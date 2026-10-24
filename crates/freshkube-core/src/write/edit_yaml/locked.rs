//! What an edit can never change, whatever text a page hands over.

/// The cluster's own fields: left out of the text the person edits, and out of
/// what is sent. `status` is the controllers', `managedFields` the server's
/// record of who owns what, and the rest the server sets itself.
pub(super) const HIDDEN: [&str; 7] = [
    "status",
    "metadata.managedFields",
    "metadata.resourceVersion",
    "metadata.uid",
    "metadata.generation",
    "metadata.creationTimestamp",
    "metadata.selfLink",
];

/// The object's identity, which an edit keeps as it is: a different name or
/// kind would be another object.
pub(super) const IDENTITY: [&str; 4] =
    ["apiVersion", "kind", "metadata.name", "metadata.namespace"];

/// What a Secret's `data` and `stringData` read in the text: its values are
/// never fetched.
pub const NOT_READ: &str = "‹not read›";

/// The fields the person is told are locked, under the editor's toolbar.
pub fn locked_fields(secret: bool) -> Vec<&'static str> {
    let mut fields = vec![
        "status",
        "metadata.managedFields",
        "metadata.uid",
        "metadata.resourceVersion",
        "apiVersion",
        "kind",
        "metadata.name",
        "metadata.namespace",
    ];
    if secret {
        fields.extend(["data", "stringData"]);
    }
    fields
}
