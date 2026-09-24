macro_rules! declare_settings_http_surface {
    (
        dto: $dto:ty,
        read_port: $read_port:ident,
        view_fn: $view_fn:ident,
        view_to_dto: $view_to_dto:path,
        watch_port: $watch_port:ident,
        watch_load: $watch_load:ident,
        http_state: $http_state:ident,
        dto_fn: $dto_fn:ident,
        apply_watch: $apply_watch:ident,
        read_bridge: $read_bridge:ident,
        watch_bridge: $watch_bridge:ident,
    ) => {
        pub trait $apply_watch: Send + Sync {
            fn $watch_load(&self) -> u32;
        }

        pub trait $http_state: Send + Sync {
            fn $dto_fn(&self) -> $dto;
        }

        pub struct $read_bridge {
            port: std::sync::Arc<dyn dali2rust_domain::registry::$read_port>,
        }

        impl $read_bridge {
            pub fn new(port: std::sync::Arc<dyn dali2rust_domain::registry::$read_port>) -> Self {
                Self { port }
            }
        }

        impl $http_state for $read_bridge {
            fn $dto_fn(&self) -> $dto {
                $view_to_dto(self.port.$view_fn())
            }
        }

        pub struct $watch_bridge {
            port: std::sync::Arc<dyn dali2rust_domain::registry::$watch_port>,
        }

        impl $watch_bridge {
            pub fn new(port: std::sync::Arc<dyn dali2rust_domain::registry::$watch_port>) -> Self {
                Self { port }
            }
        }

        impl $apply_watch for $watch_bridge {
            fn $watch_load(&self) -> u32 {
                self.port.$watch_load()
            }
        }
    };
}

pub(crate) use declare_settings_http_surface;
