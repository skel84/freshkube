//! `get`'s version 1 JSON for the kinds that existed before `changes` and
//! `waiting-to-ship` came: byte for byte, so a consumer written against slice
//! 1 keeps working. Fields are only ever added, never renamed or moved.

use super::*;

const SECRETS: &str = r#"{"version":1,"command":"get","cluster":{"entry":null,"context":"acme-prod"},"exit":0,"kind":{"key":"secrets","kind":"Secret","namespaced":true},"namespace":null,"columns":["Name","Type","Data","Age"],"items":[{"namespace":"shop","name":"db-credentials","cells":{"Name":"db-credentials","Type":"Opaque","Data":"2","Age":"2d"},"state":null,"reason":null,"view":{"page":"resources","kind":"secrets","namespace":"shop","name":"db-credentials"}}],"problems":[]}
"#;

const PODS: &str = r#"{"version":1,"command":"get","cluster":{"entry":null,"context":"acme-prod"},"exit":0,"kind":{"key":"pods","kind":"Pod","namespaced":true},"namespace":null,"columns":["Name","Ready","Status","Restarts","Age"],"items":[{"namespace":"shop","name":"api-1","cells":{"Name":"api-1","Ready":"1/1","Status":"Running","Restarts":"0","Age":"2d"},"state":"good","reason":"Running","view":{"page":"resources","kind":"pods","namespace":"shop","name":"api-1"}},{"namespace":"shop","name":"job-1","cells":{"Name":"job-1","Ready":"0/1","Status":"Completed","Restarts":"0","Age":"2d"},"state":"good","reason":"Completed","view":{"page":"resources","kind":"pods","namespace":"shop","name":"job-1"}}],"problems":[]}
"#;

#[tokio::test]
async fn get_pods_json_is_byte_identical_to_version_1() {
    let fake = Fake::answering(healthy());
    let result = run(&fake, &format!("get pods {KUBE} --json")).await;
    assert_eq!(result.code, 0, "{}", result.err);
    assert_eq!(result.out, PODS);
}

#[tokio::test]
async fn get_secrets_json_is_byte_identical_to_version_1() {
    let fake = Fake::answering(healthy());
    let result = run(&fake, &format!("get secrets {KUBE} --json")).await;
    assert_eq!(result.code, 0, "{}", result.err);
    assert_eq!(result.out, SECRETS);
}
