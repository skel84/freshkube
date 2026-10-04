//! An application's profiles, from Coroot's profiling view: the profile types
//! it has, and one flame graph, optionally compared with the period before.
//!
//! A flame graph nests a JSON object and an array per stack frame, deeper
//! than serde_json's default limit, so this view is read as bytes, its depth
//! checked, and parsed and flattened on a thread with a large stack.
use super::*;
use serde::Deserialize;

/// Frames narrower than this share of the whole are left out.
const MIN_SHARE: f64 = 0.0005;
/// Frames kept, shallowest first.
const MAX_FRAMES: usize = 20_000;
/// JSON nesting accepted: two levels per stack frame.
const MAX_NESTING: usize = 4_096;
/// Stack for parsing and dropping a deep flame graph.
const PARSE_STACK: usize = 256 * 1024 * 1024;

/// Which profile to read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ProfileQuery {
    /// A type Coroot listed, such as `go:profile_cpu:nanoseconds`; none
    /// lets Coroot choose the application's main CPU profile.
    pub kind: Option<String>,
    /// Compare the window with the one before it.
    pub compare: bool,
    /// One instance (pod) instead of every instance.
    pub instance: Option<String>,
}

impl ProfileQuery {
    fn param(&self) -> Result<String, ReadError> {
        let Some(kind) = &self.kind else {
            return Ok("cpu".into());
        };
        let instance = self.instance.as_deref().unwrap_or_default();
        if kind.is_empty() || kind.len() > 128 || instance.len() > 256 {
            return Err(ReadError::InvalidSelection);
        }
        Ok(serde_json::json!({
            "type": kind,
            "mode": if self.compare { "diff" } else { "" },
            "instance": instance,
        })
        .to_string())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Profiling {
    /// `Unknown` when Coroot found no profiles, `Warning` when it could not read them.
    pub status: Status,
    /// Coroot's note, as plain text.
    pub message: String,
    /// Profile types for this application, by name.
    pub kinds: Vec<ProfileKind>,
    /// The application's instances, for narrowing the profile to one.
    pub instances: Vec<String>,
    pub graph: Option<FlameGraph>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKind {
    pub id: String,
    pub name: String,
}

impl ProfileKind {
    /// What a frame's value counts, from the type's last part.
    pub fn unit(id: &str) -> ProfileUnit {
        match id.rsplit(':').next() {
            Some("nanoseconds") => ProfileUnit::Nanoseconds,
            Some("bytes") => ProfileUnit::Bytes,
            _ => ProfileUnit::Count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileUnit {
    Nanoseconds,
    Bytes,
    Count,
}

#[derive(Clone, Debug)]
pub struct FlameGraph {
    pub kind: String,
    pub compared: bool,
    /// The root first, then breadth first; a frame's parent comes before it.
    pub frames: Vec<Frame>,
    /// Frames left out as too narrow to draw or past the frame limit.
    pub omitted: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub name: String,
    pub parent: Option<usize>,
    pub depth: usize,
    /// Left edge and width, as shares of the root.
    pub x: f64,
    pub width: f64,
    pub total: i64,
    pub self_value: i64,
    /// How much the frame's share changed from the previous window to this
    /// one, in percentage points; none without a comparison.
    pub change: Option<f64>,
}

#[derive(Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    data: Option<RawView>,
}

#[derive(Deserialize)]
struct RawView {
    #[serde(default)]
    status: Status,
    #[serde(default)]
    message: String,
    #[serde(default)]
    profiles: Option<Vec<RawKind>>,
    #[serde(default)]
    instances: Option<Vec<String>>,
    #[serde(default)]
    profile: Option<RawProfile>,
}

#[derive(Deserialize)]
struct RawKind {
    #[serde(rename = "type")]
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct RawProfile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    diff: bool,
    #[serde(default)]
    flamegraph: Option<Node>,
}

#[derive(Deserialize)]
struct Node {
    name: String,
    #[serde(default)]
    total: i64,
    #[serde(default, rename = "self")]
    self_value: i64,
    #[serde(default)]
    comp: i64,
    #[serde(default)]
    children: Option<Vec<Node>>,
}

/// The deepest JSON nesting in `body`, stopping once it passes `limit`.
fn nesting(body: &[u8], limit: usize) -> usize {
    let (mut depth, mut deepest) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for &b in body {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                deepest = deepest.max(depth);
                if deepest > limit {
                    return deepest;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

pub(super) fn decode(body: Vec<u8>) -> Result<Profiling, ReadError> {
    if nesting(&body, MAX_NESTING) > MAX_NESTING {
        return Err(ReadError::Limit);
    }
    // Deserializing and dropping the tree recurse once per level.
    std::thread::Builder::new()
        .name("coroot-profile".into())
        .stack_size(PARSE_STACK)
        .spawn(move || decode_deep(&body))
        .map_err(|_| ReadError::Failed)?
        .join()
        .map_err(|_| ReadError::InvalidResponse)?
}

fn decode_deep(body: &[u8]) -> Result<Profiling, ReadError> {
    let mut de = serde_json::Deserializer::from_slice(body);
    de.disable_recursion_limit();
    let raw = RawEnvelope::deserialize(&mut de).map_err(|_| ReadError::InvalidResponse)?;
    let Some(view) = raw.data else {
        return Err(ReadError::Missing);
    };
    let graph = view.profile.and_then(|profile| {
        let root = profile.flamegraph?;
        Some(flatten(profile.kind, profile.diff, &root))
    });
    Ok(Profiling {
        status: view.status,
        message: tracing::plain(&view.message),
        kinds: view
            .profiles
            .unwrap_or_default()
            .into_iter()
            .map(|k| ProfileKind {
                id: k.id,
                name: k.name,
            })
            .collect(),
        instances: view.instances.unwrap_or_default(),
        graph,
    })
}

/// Lays the tree out breadth first, dropping frames too narrow to see.
fn flatten(kind: String, compared: bool, root: &Node) -> FlameGraph {
    let whole = root.total.max(1) as f64;
    // In a comparison `total` counts both windows and `comp` the current one.
    let (current, previous) = (root.comp, root.total - root.comp);
    let change = |node: &Node| {
        (compared && current > 0 && previous > 0).then(|| {
            let now = node.comp as f64 / current as f64;
            let before = (node.total - node.comp) as f64 / previous as f64;
            (now - before) * 100.
        })
    };
    let mut frames = vec![Frame {
        name: clip(&root.name, "total"),
        parent: None,
        depth: 0,
        x: 0.,
        width: 1.,
        total: root.total,
        self_value: root.self_value,
        change: change(root),
    }];
    let mut omitted = 0;
    let mut queue = std::collections::VecDeque::from([(root, 0usize)]);
    while let Some((node, index)) = queue.pop_front() {
        let (mut x, depth) = (frames[index].x, frames[index].depth + 1);
        for child in node.children.iter().flatten() {
            let width = child.total.max(0) as f64 / whole;
            if width < MIN_SHARE || frames.len() >= MAX_FRAMES {
                omitted += 1 + descendants(child);
            } else {
                frames.push(Frame {
                    name: clip(&child.name, "?"),
                    parent: Some(index),
                    depth,
                    x,
                    width,
                    total: child.total,
                    self_value: child.self_value,
                    change: change(child),
                });
                queue.push_back((child, frames.len() - 1));
            }
            x += width;
        }
    }
    FlameGraph {
        kind,
        compared,
        frames,
        omitted,
    }
}

fn descendants(node: &Node) -> usize {
    let mut count = 0;
    let mut stack: Vec<&Node> = node.children.iter().flatten().collect();
    while let Some(next) = stack.pop() {
        count += 1;
        stack.extend(next.children.iter().flatten());
    }
    count
}

/// Frame names can be whole C++ or Java signatures.
fn clip(name: &str, empty: &str) -> String {
    if name.is_empty() {
        return empty.into();
    }
    match name.char_indices().nth(512) {
        Some((at, _)) => format!("{}…", &name[..at]),
        None => name.into(),
    }
}

impl Provider {
    /// An application's profile in `range`.
    pub async fn profiling(
        &self,
        source: &Source,
        range: TimeRange,
        app: &AppId,
        query: &ProfileQuery,
    ) -> Result<Profiling, ReadError> {
        self.project(source, range)?;
        let param = query.param()?;
        let path = format!(
            "api/project/{}/app/{}/profiling",
            coroot_rs::util::encode_segment(source.project()),
            coroot_rs::util::encode_segment(app.as_str())
        );
        let mut params = range.query();
        params.push(("query", param));
        let client = self.client.clone();
        let body = self
            .read(async move {
                let request = client
                    .request(coroot_rs::Method::GET, &path)?
                    .query(&params);
                let response = client.execute(request).await?;
                client.read_body(response).await
            })
            .await?;
        let profiling = tokio::task::spawn_blocking(move || decode(body))
            .await
            .map_err(|_| ReadError::Failed)??;
        limits::profiling(&profiling)?;
        Ok(profiling)
    }
}
