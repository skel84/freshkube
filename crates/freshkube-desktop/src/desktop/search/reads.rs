use super::*;
impl Search {
    pub(super) fn begin(
        &mut self,
        local: Vec<Entry>,
        source: Option<KubeSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = source.as_ref().map(|source| source.id.clone());
        let reuse = self.source_id == id
            && self.closed_at.is_some_and(|closed| {
                cx.background_executor()
                    .now()
                    .saturating_duration_since(closed)
                    < Duration::from_secs(30)
            });
        self.jobs.clear();
        self.sequence = self.sequence.wrapping_add(1);
        if !reuse {
            self.parts.clear();
        }
        self.source_id = id;
        self.local = local;
        self.query.clear();
        self.open = true;
        self.closed_at = None;
        self.command
            .update(cx, |state, cx| state.set_query("", window, cx));
        if let Some(source) = source {
            for key in KINDS {
                if self.parts.contains_key(key) {
                    continue;
                }
                let kind = builtin(key).unwrap();
                if matches!(source.access, KubeAccess::Example) {
                    let objects = resources::example::read(
                        &source.context,
                        key,
                        None,
                        chrono::Utc::now().timestamp(),
                    )
                    .map(|(_, rows)| {
                        rows.into_iter()
                            .take(2_000)
                            .map(|row| row.identity.into())
                            .collect()
                    })
                    .unwrap_or_default();
                    self.parts.insert(
                        key,
                        Ok(Part {
                            entries: model::object_entries(kind, objects),
                            capped: false,
                        }),
                    );
                    continue;
                }
                let access = source.access.clone();
                let connection = source.id.clone();
                let sequence = self.sequence;
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    Duration::from_secs(5),
                    format!("Listing {key} timed out"),
                    async move {
                        let client = access.client().await?;
                        let metadata = freshkube_core::resources::list_metadata(&client, &kind)
                            .await
                            .map_err(|error| error.to_string())?;
                        let objects = metadata
                            .objects
                            .into_iter()
                            .filter_map(|object| {
                                Some(ObjectRef {
                                    name: object.name?,
                                    namespace: object.namespace.unwrap_or_default(),
                                    uid: object.uid?,
                                    connection: Some(connection.clone()),
                                })
                            })
                            .collect();
                        Ok(Part {
                            entries: model::object_entries(kind, objects),
                            capped: metadata.capped,
                        })
                    },
                );
                let task = cx.spawn(async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("The metadata worker stopped".into()));
                    _ = this.update(cx, |search, cx| search.answer(sequence, key, result, cx));
                });
                self.jobs.push((job, task));
            }
        } else {
            for key in KINDS {
                self.parts
                    .insert(key, Err("Kubernetes is unavailable".into()));
            }
        }
        self.rebuild();
        cx.notify();
    }
    pub(super) fn answer(
        &mut self,
        sequence: u64,
        key: &'static str,
        result: Result<Part, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.open || self.sequence != sequence {
            return;
        }
        self.parts.insert(key, result);
        self.rebuild();
        cx.notify();
    }
}
