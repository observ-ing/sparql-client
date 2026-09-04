mod mock_server;

use mock_server::{MockServer, Reply};
use sparql_client::{Error, SparqlClient};
use std::time::Duration;

const QUERY: &str = "SELECT ?item WHERE { ?item wdt:P31 wd:Q5 } LIMIT 1";
const ROWS: &str = r#"{"head":{"vars":["item"]},"results":{"bindings":[{"item":{"type":"uri","value":"http://www.wikidata.org/entity/Q42"}}]}}"#;
const USER_AGENT: &str = "sparqling-tests/0.1 (tests@example.org)";

fn client(server: &MockServer, max_retries: u32) -> SparqlClient {
    SparqlClient::builder(server.url())
        .user_agent(USER_AGENT)
        .timeout(Duration::from_millis(200))
        .max_retries(max_retries)
        .retry_base_delay(Duration::from_millis(1))
        .build()
        .unwrap()
}

#[tokio::test]
async fn posts_the_query_with_the_protocol_headers() {
    let server = MockServer::start(vec![Reply::json(200, ROWS)]);

    let rows = client(&server, 0).sparql_query(QUERY).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["item"].value, "http://www.wikidata.org/entity/Q42");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/sparql");
    assert_eq!(
        request.header("content-type"),
        Some("application/sparql-query")
    );
    assert_eq!(
        request.header("accept"),
        Some("application/sparql-results+json")
    );
    assert_eq!(request.header("user-agent"), Some(USER_AGENT));
    assert_eq!(request.body, QUERY);
}

#[tokio::test]
async fn answers_ask_queries() {
    let server = MockServer::start(vec![Reply::json(200, r#"{"head":{},"boolean":true}"#)]);

    assert!(client(&server, 0)
        .sparql_ask("ASK { ?s ?p ?o }")
        .await
        .unwrap());
}

#[tokio::test]
async fn retries_a_throttled_request_after_retry_after() {
    let server = MockServer::start(vec![
        Reply::text(429, "slow down").header("Retry-After", "0"),
        Reply::json(200, ROWS),
    ]);

    let rows = client(&server, 1).sparql_query(QUERY).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn backs_off_and_retries_after_503() {
    let server = MockServer::start(vec![
        Reply::text(503, "maintenance"),
        Reply::json(200, ROWS),
    ]);

    let rows = client(&server, 1).sparql_query(QUERY).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start(vec![
        Reply::text(429, "slow down"),
        Reply::text(429, "still busy"),
    ]);

    let error = client(&server, 1).sparql_query(QUERY).await.unwrap_err();

    assert!(error.is_throttled());
    assert!(
        matches!(&error, Error::Status { status, body } if status.as_u16() == 429 && body == "still busy")
    );
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn does_not_retry_a_client_error() {
    let server = MockServer::start(vec![Reply::text(400, "Malformed query")]);

    let error = client(&server, 2).sparql_query(QUERY).await.unwrap_err();

    assert!(
        matches!(&error, Error::Status { status, body } if status.as_u16() == 400 && body == "Malformed query")
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn retries_a_timed_out_request() {
    let server = MockServer::start(vec![
        Reply::json(200, ROWS).delayed(Duration::from_secs(2)),
        Reply::json(200, ROWS),
    ]);

    let rows = client(&server, 1).sparql_query(QUERY).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn does_not_retry_a_timeout_without_retries() {
    let server = MockServer::start(vec![Reply::json(200, ROWS).delayed(Duration::from_secs(2))]);

    let error = client(&server, 0).sparql_query(QUERY).await.unwrap_err();

    assert!(error.is_timeout());
}

#[tokio::test]
async fn reports_an_undecodable_body() {
    let server = MockServer::start(vec![Reply::json(200, "not json")]);

    let error = client(&server, 0).sparql_query(QUERY).await.unwrap_err();

    assert!(matches!(error, Error::Decode(_)));
}
