use crate::http::handlers::common::require_get;
use crate::http::handlers::resource_surface::declare_handler_shell;
use std::collections::HashMap;

use crate::http::handler::ApiHandler;
use crate::http::types::{HttpBody, HttpResponse};

pub const INDEX_ROUTE_PATH: &str = "/";
const API_PREFIX: &str = "/api/";
const HASHED_ASSET_PREFIX: &str = "/assets/";

pub const HEADER_CONTENT_ENCODING_GZIP: (&str, &str) = ("Content-Encoding", "gzip");
pub const CACHE_CONTROL_NO_CACHE: (&str, &str) = ("Cache-Control", "no-cache");
pub const CACHE_CONTROL_HASHED: (&str, &str) =
    ("Cache-Control", "max-age=31536000, immutable");

const NO_CACHE_HEADERS: &[(&str, &str)] = &[HEADER_CONTENT_ENCODING_GZIP, CACHE_CONTROL_NO_CACHE];
const HASHED_HEADERS: &[(&str, &str)] = &[HEADER_CONTENT_ENCODING_GZIP, CACHE_CONTROL_HASHED];

#[derive(Debug, Clone, Copy)]
pub struct StaticAsset {
    pub route_path: &'static str,
    pub content_type: &'static str,
    pub gzip_body: &'static [u8],
}

declare_handler_shell!(StaticAssetHandler {
    assets: &'static [StaticAsset],
});

impl StaticAssetHandler {

    fn lookup(&self, path: &str) -> Option<&StaticAsset> {
        self.assets.iter().find(|a| a.route_path == path)
    }

    fn asset_response(asset: &StaticAsset) -> HttpResponse {
        let extra_headers = if asset.route_path.starts_with(HASHED_ASSET_PREFIX) {
            HASHED_HEADERS
        } else {
            NO_CACHE_HEADERS
        };
        HttpResponse {
            status: 200,
            content_type: asset.content_type,
            extra_headers,
            body: HttpBody::Stream(Box::new({
                let bytes = asset.gzip_body;
                move |w| w.write_all(bytes)
            })),
        }
    }
}

impl ApiHandler for StaticAssetHandler {
    fn handle_request(
        &self,
        method: &str,
        path: &str,
        _body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if let Err(error) = require_get(method) {
            return error;
        }
        if path.starts_with(API_PREFIX) {
            return HttpResponse::not_found();
        }
        match self.lookup(path).or_else(|| self.lookup(INDEX_ROUTE_PATH)) {
            Some(asset) => Self::asset_response(asset),
            None => HttpResponse::not_found(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GZ_INDEX: &[u8] = &[0x1f, 0x8b, 1, 2, 3];
    const GZ_APP: &[u8] = &[0x1f, 0x8b, 9, 8, 7];
    const GZ_ICON: &[u8] = &[0x1f, 0x8b, 4, 5, 6];
    const ASSETS: &[StaticAsset] = &[
        StaticAsset {
            route_path: "/",
            content_type: "text/html",
            gzip_body: GZ_INDEX,
        },
        StaticAsset {
            route_path: "/assets/app.js",
            content_type: "application/javascript",
            gzip_body: GZ_APP,
        },
        StaticAsset {
            route_path: "/favicon.svg",
            content_type: "image/svg+xml",
            gzip_body: GZ_ICON,
        },
    ];

    fn cache_control(res: &HttpResponse) -> &'static str {
        res.extra_headers
            .iter()
            .find(|(k, _)| *k == "Cache-Control")
            .expect("Cache-Control header")
            .1
    }

    fn handle(path: &str) -> HttpResponse {
        StaticAssetHandler::new(ASSETS).handle_request("GET", path, &[], &HashMap::new())
    }

    #[test]
    fn serves_index_with_gzip_and_no_cache() {
        let res = handle("/");
        assert_eq!(res.status, 200);
        assert_eq!(res.content_type, "text/html");
        assert!(res.extra_headers.contains(&HEADER_CONTENT_ENCODING_GZIP));
        assert_eq!(cache_control(&res), "no-cache");
        assert_eq!(res.into_body_bytes(), GZ_INDEX);
    }

    #[test]
    fn serves_hashed_asset_with_gzip_and_immutable_cache() {
        let res = handle("/assets/app.js");
        assert_eq!(res.status, 200);
        assert_eq!(res.content_type, "application/javascript");
        assert!(res.extra_headers.contains(&HEADER_CONTENT_ENCODING_GZIP));
        assert_eq!(cache_control(&res), "max-age=31536000, immutable");
        assert_eq!(res.into_body_bytes(), GZ_APP);
    }

    #[test]
    fn unhashed_asset_outside_assets_dir_is_not_cached() {
        let res = handle("/favicon.svg");
        assert_eq!(res.status, 200);
        assert_eq!(cache_control(&res), "no-cache");
        assert_eq!(res.into_body_bytes(), GZ_ICON);
    }

    #[test]
    fn unknown_non_api_path_falls_back_to_index() {
        let res = handle("/some/client/route");
        assert_eq!(res.status, 200);
        assert_eq!(res.content_type, "text/html");
        assert_eq!(cache_control(&res), "no-cache");
        assert_eq!(res.into_body_bytes(), GZ_INDEX);
    }

    #[test]
    fn api_prefix_keeps_json_not_found() {
        let res = handle("/api/v1/nonexistent");
        assert_eq!(res.status, 404);
        assert_eq!(res.content_type, "application/json");
    }

    #[test]
    fn non_get_method_is_rejected() {
        let res = StaticAssetHandler::new(ASSETS).handle_request("POST", "/", &[], &HashMap::new());
        assert_eq!(res.status, 405);
    }
}
