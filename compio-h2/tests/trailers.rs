//! Trailer tests — sending and receiving HTTP/2 trailers.

mod common;

#[compio_macros::test]
async fn client_sends_trailers() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/trailers"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();
    ss.send_data(b"body data".to_vec(), false).await.unwrap();

    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-checksum", "abc123".parse().unwrap());
    ss.send_trailers(trailers).await.unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (_, mut recv) = req.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"body data");

    let trailers = recv
        .trailers()
        .await
        .expect("trailers should be present")
        .expect("trailers should decode successfully");
    assert_eq!(
        trailers.get("x-checksum").unwrap().to_str().unwrap(),
        "abc123"
    );

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn server_sends_trailers() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, _) = client
        .send_request(common::get_request("/srv-trailers"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let mut resp_stream = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    resp_stream
        .send_data(b"server body".to_vec(), false)
        .await
        .unwrap();

    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-trailer", "trailer-value".parse().unwrap());
    resp_stream.send_trailers(trailers).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let (_, mut recv) = resp.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"server body");

    let trailers = recv
        .trailers()
        .await
        .expect("trailers should be present")
        .expect("trailers should decode successfully");
    assert_eq!(
        trailers.get("x-trailer").unwrap().to_str().unwrap(),
        "trailer-value"
    );
}

#[compio_macros::test]
async fn trailers_without_body() {
    let (mut client, mut server) = common::setup().await;

    // Client sends headers only (no body)
    let (resp_fut, _) = client
        .send_request(common::get_request("/trailers-no-body"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();

    // Server sends body + trailers
    let mut resp_stream = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    resp_stream
        .send_data(b"with trailers".to_vec(), false)
        .await
        .unwrap();

    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-end", "done".parse().unwrap());
    resp_stream.send_trailers(trailers).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let (_, mut recv) = resp.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"with trailers");

    let trailers = recv
        .trailers()
        .await
        .expect("trailers should be present")
        .expect("trailers should decode successfully");
    assert_eq!(trailers.get("x-end").unwrap().to_str().unwrap(), "done");
}
