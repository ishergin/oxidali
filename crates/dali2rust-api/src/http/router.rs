use super::handler::ApiHandler;
use super::types::{HttpMethod, HttpResponse, RouteRegisterError};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestBody {
    Json,
    Raw,
}

pub struct RouteSpec {
    pub method: HttpMethod,
    pub path: &'static str,
    pub handler: Box<dyn ApiHandler>,
    pub body: RequestBody,
}

struct Route {
    handler: Box<dyn ApiHandler>,
    body: RequestBody,
}

impl RouteSpec {
    pub fn new(method: HttpMethod, path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self {
            method,
            path,
            handler,
            body: RequestBody::Json,
        }
    }

    pub fn raw_body(mut self) -> Self {
        self.body = RequestBody::Raw;
        self
    }

    pub fn get(path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self::new(HttpMethod::Get, path, handler)
    }

    pub fn post(path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self::new(HttpMethod::Post, path, handler)
    }

    pub fn patch(path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self::new(HttpMethod::Patch, path, handler)
    }

    pub fn put(path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self::new(HttpMethod::Put, path, handler)
    }

    pub fn delete(path: &'static str, handler: Box<dyn ApiHandler>) -> Self {
        Self::new(HttpMethod::Delete, path, handler)
    }
}

pub struct Router {
    routers: HashMap<HttpMethod, matchit::Router<Route>>,
    role: Option<std::sync::Arc<dyn crate::http::role::ControllerRolePort>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routers: HashMap::new(),
            role: None,
        }
    }

    pub fn with_role_port(
        mut self,
        role: std::sync::Arc<dyn crate::http::role::ControllerRolePort>,
    ) -> Self {
        self.role = Some(role);
        self
    }

    fn stamped(&self, mut response: HttpResponse) -> HttpResponse {
        if let Some(role) = self.role.as_ref() {
            response.extra_headers =
                crate::http::role::role_headers(response.extra_headers, role.is_active());
        }
        response
    }

    fn router_for(&mut self, method: HttpMethod) -> &mut matchit::Router<Route> {
        self.routers.entry(method).or_default()
    }

    pub fn register(&mut self, spec: RouteSpec) -> Result<(), RouteRegisterError> {
        let router = self.router_for(spec.method);
        let route = Route {
            handler: spec.handler,
            body: spec.body,
        };
        router
            .insert(spec.path, route)
            .map_err(|e| RouteRegisterError(e.to_string()))?;
        Ok(())
    }

    pub fn dispatch(&self, method: &str, uri: &str, body: &[u8]) -> HttpResponse {
        let (path, query) = match uri.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (uri, None),
        };
        let http_method = HttpMethod::from_str(method);
        match self.routers.get(&http_method) {
            Some(router) => match router.at(path) {
                Ok(m) => {
                    let mut params: HashMap<String, String> =
                        query.map(parse_query).unwrap_or_default();
                    params.extend(
                        m.params
                            .iter()
                            .map(|(k, v)| (k.to_string(), percent_decode_path(v))),
                    );
                    self.stamped(m.value.handle(method, path, body, &params))
                }
                Err(_) => self.stamped(HttpResponse::not_found()),
            },
            None => self.stamped(HttpResponse::not_found()),
        }
    }
}

impl Route {
    fn handle(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        params: &HashMap<String, String>,
    ) -> HttpResponse {
        if self.body == RequestBody::Json && crate::json_depth::json_too_deep(body) {
            return crate::http::handlers::common::json_err(400, "invalid_json");
        }
        self.handler.handle_request(method, path, body, params)
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    decode(s, true)
}

fn percent_decode_path(s: &str) -> String {
    decode(s, false)
}

fn decode(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OkHandler;
    impl ApiHandler for OkHandler {
        fn handle_request(
            &self,
            _method: &str,
            _path: &str,
            _body: &[u8],
            _params: &HashMap<String, String>,
        ) -> HttpResponse {
            HttpResponse::json(200, br#"{"ok":true}"#.to_vec())
        }
    }


    fn body_text(body: crate::http::types::HttpBody) -> String {
        String::from_utf8(body.into_bytes()).expect("utf-8")
    }

    struct EchoParamsHandler;
    impl ApiHandler for EchoParamsHandler {
        fn handle_request(
            &self,
            _method: &str,
            _path: &str,
            _body: &[u8],
            params: &HashMap<String, String>,
        ) -> HttpResponse {
            let mut sorted: Vec<_> = params.iter().collect();
            sorted.sort_by_key(|(k, _)| k.as_str());
            let body = format!("{:?}", sorted);
            HttpResponse::json(200, body.into_bytes())
        }
    }

    #[test]
    fn register_and_dispatch() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/health", Box::new(OkHandler)))
            .unwrap();
        let resp = router.dispatch("GET", "/api/v1/health", &[]);
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn method_mismatch_returns_404() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/health", Box::new(OkHandler)))
            .unwrap();
        let resp = router.dispatch("POST", "/api/v1/health", &[]);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn unknown_route_returns_404() {
        let router = Router::new();
        let resp = router.dispatch("GET", "/api/v1/nonexistent", &[]);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn duplicate_route_fails() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/health", Box::new(OkHandler)))
            .unwrap();
        let result = router.register(RouteSpec::get("/api/v1/health", Box::new(OkHandler)));
        assert!(result.is_err());
    }

    #[test]
    fn strips_query_string() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/health", Box::new(OkHandler)))
            .unwrap();
        let resp = router.dispatch("GET", "/api/v1/health?foo=bar", &[]);
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn path_params_extracted() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get(
                "/api/v1/devices/{id}",
                Box::new(EchoParamsHandler),
            ))
            .unwrap();
        let resp = router.dispatch("GET", "/api/v1/devices/42", &[]);
        assert_eq!(resp.status, 200);
        let bytes = resp.into_body_bytes();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("\"id\""));
        assert!(body.contains("\"42\""));
    }

    #[test]
    fn multiple_path_params() {
        let mut router = Router::new();
        router
            .register(RouteSpec::post(
                "/api/v1/devices/{id}/control",
                Box::new(EchoParamsHandler),
            ))
            .unwrap();
        let resp = router.dispatch("POST", "/api/v1/devices/7/control", &[]);
        assert_eq!(resp.status, 200);
        let bytes = resp.into_body_bytes();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(body.contains("\"id\""));
        assert!(body.contains("\"7\""));
    }

    #[test]
    fn same_path_different_methods() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/devices", Box::new(OkHandler)))
            .unwrap();
        router
            .register(RouteSpec::post("/api/v1/devices", Box::new(OkHandler)))
            .unwrap();
        let get_resp = router.dispatch("GET", "/api/v1/devices", &[]);
        let post_resp = router.dispatch("POST", "/api/v1/devices", &[]);
        assert_eq!(get_resp.status, 200);
        assert_eq!(post_resp.status, 200);
        let delete_resp = router.dispatch("DELETE", "/api/v1/devices", &[]);
        assert_eq!(delete_resp.status, 404);
    }

    #[test]
    fn a_path_parameter_is_percent_decoded() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/rules/{name}", Box::new(EchoParamsHandler)))
            .expect("route");
        let encoded = "/api/v1/rules/%D0%BA%D0%BD%D0%BE%D0%BF%D0%BA%D0%B0%201";
        let resp = router.dispatch("GET", encoded, &[]);
        assert_eq!(resp.status, 200);
        let body = body_text(resp.body);
        assert!(body.contains("кнопка 1"), "path param not decoded: {body}");
    }

    #[test]
    fn a_plus_is_a_space_in_a_query_and_a_plus_in_a_path() {
        let mut router = Router::new();
        router
            .register(RouteSpec::get("/api/v1/rules/{name}", Box::new(EchoParamsHandler)))
            .expect("route");
        let resp = router.dispatch("GET", "/api/v1/rules/day+night?q=a+b", &[]);
        let body = body_text(resp.body);
        assert!(body.contains("day+night"), "path plus was eaten: {body}");
        assert!(body.contains("a b"), "query plus was not a space: {body}");
    }
}
