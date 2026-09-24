use dali2rust_adapters::StaticAsset;

macro_rules! asset_bytes {
    ($rel:literal) => {
        include_bytes!(concat!("../assets/web/", $rel))
    };
}

pub const WEB_ASSETS: &[StaticAsset] = &[
    StaticAsset {
        route_path: "/",
        content_type: "text/html",
        gzip_body: asset_bytes!("index.html.gz"),
    },
    StaticAsset {
        route_path: "/assets/app.js",
        content_type: "application/javascript",
        gzip_body: asset_bytes!("assets/app.js.gz"),
    },
    StaticAsset {
        route_path: "/assets/app.css",
        content_type: "text/css",
        gzip_body: asset_bytes!("assets/app.css.gz"),
    },
    StaticAsset {
        route_path: "/favicon.svg",
        content_type: "image/svg+xml",
        gzip_body: asset_bytes!("favicon.svg.gz"),
    },
    StaticAsset {
        route_path: "/dali-products.json",
        content_type: "application/json",
        gzip_body: include_bytes!(env!("PRODUCTS_DB_GZ")),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_paths_are_absolute_and_not_gz_suffixed() {
        for asset in WEB_ASSETS {
            assert!(asset.route_path.starts_with('/'), "{}", asset.route_path);
            assert!(!asset.route_path.ends_with(".gz"), "{}", asset.route_path);
        }
    }

    #[test]
    fn index_asset_is_present() {
        assert!(WEB_ASSETS.iter().any(|a| a.route_path == "/"));
    }

    #[test]
    fn every_body_carries_the_gzip_magic() {
        for asset in WEB_ASSETS {
            assert_eq!(
                &asset.gzip_body[..2],
                &[0x1f, 0x8b],
                "{} is not gzip",
                asset.route_path
            );
        }
    }
}
