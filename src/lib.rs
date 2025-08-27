use spin_sdk::variables;
use uuid::Uuid;
use wasi::{
    exports::http::incoming_handler::Guest,
    http::{
        outgoing_handler,
        types::{
            FieldValue, Headers, IncomingBody, Method, OutgoingBody, OutgoingRequest,
            OutgoingResponse, ResponseOutparam, Scheme,
        },
    },
};

struct Component;
wasi::http::proxy::export!(Component);

fn internal_server_error(
    response_out: wasi::exports::http::incoming_handler::ResponseOutparam,
) -> () {
    let error_res = OutgoingResponse::new(Headers::new());
    error_res
        .set_status_code(500)
        .expect("Could not set status on error response");
    ResponseOutparam::set(response_out, Ok(error_res))
}
impl Guest for Component {
    fn handle(
        request: wasi::exports::http::incoming_handler::IncomingRequest,
        response_out: wasi::exports::http::incoming_handler::ResponseOutparam,
    ) -> () {
        let Ok(origin) = variables::get("origin") else {
            return internal_server_error(response_out);
        };
        let Ok(header_name) = variables::get("header_name") else {
            return internal_server_error(response_out);
        };

        let headers = Headers::from(request.headers().clone());
        headers
            .set(
                &header_name,
                &vec![Uuid::new_v4().to_string().as_bytes().to_vec()],
            )
            .expect("Could not set correlation ID on upstream request");

        let upstream_req = OutgoingRequest::new(headers);
        upstream_req
            .set_method(&request.method())
            .expect("Could not set method on upstream request");
        upstream_req
            .set_authority(Some(origin.as_str()))
            .expect("Coult not set authority on upstream request");
        upstream_req
            .set_scheme(Some(&Scheme::Https))
            .expect("Could not set scheme on upstream request");
        let path = request
            .path_with_query()
            .expect("Could not read path (including query) from inbound request");
        upstream_req
            .set_path_with_query(Some(path.as_str()))
            .expect("Could not set path (including query) on upstream request");
        let mut outgoing_body = None;
        if has_body(
            request.method(),
            request.headers().get(&String::from("Content-Length")),
        ) {
            outgoing_body = Some(
                upstream_req
                    .body()
                    .expect("Could not accquire upstream request body resouce"),
            );
        }

        let upstream_res = outgoing_handler::handle(upstream_req, None)
            .expect("Could not dispatch uptream request");

        if let Some(outgoing_body) = outgoing_body {
            let incoming_body = request
                .consume()
                .expect("Could not consume incoming request body");
            forward_body(&incoming_body, outgoing_body);
        }

        upstream_res.subscribe().block();
        let incoming_upstream_res = upstream_res
            .get()
            .expect("Upstream response future not ready")
            .expect("Subsequent upstream response acceess")
            .expect("Could not collect upstream response");

        let res = OutgoingResponse::new(incoming_upstream_res.headers().clone());
        let incoming_upstream_body = incoming_upstream_res
            .consume()
            .expect("Could not consume upstream response body");
        let res_body = res
            .body()
            .expect("Could not create outgoing response body resource");
        forward_body(&incoming_upstream_body, res_body);
        ResponseOutparam::set(response_out, Ok(res));
    }
}

fn forward_body(incoming: &IncomingBody, outgoing: OutgoingBody) {
    {
        let in_stream = incoming
            .stream()
            .expect("Could not create read stream from source");
        let out_stream = outgoing
            .write()
            .expect("Could not create write stream for destination");
        out_stream
            .blocking_splice(&in_stream, u64::MAX)
            .expect("Could not splice streams");
    }
    OutgoingBody::finish(outgoing, None).expect("Could not finish body at destination");
}

fn has_body(method: Method, content_length: Vec<FieldValue>) -> bool {
    if !matches!(method, Method::Post | Method::Put | Method::Patch) {
        return false;
    }
    content_length
        .iter()
        .filter_map(|val| std::str::from_utf8(val).ok())
        .filter_map(|s| s.parse::<u64>().ok())
        .any(|len| len > 0)
}
