macro_rules! declare_read_handler {
    (@shell $(#[$meta:meta])* $handler:ident, $state:ident) => {
        $(#[$meta])*
        pub struct $handler {
            state: std::sync::Arc<dyn $state>,
        }

        impl $handler {
            pub fn new(state: std::sync::Arc<dyn $state>) -> Self {
                Self { state }
            }
        }
    };
    (@get $handler:ident, |$port:ident, $adapter:ident, $params:ident| $body:expr) => {
        impl $crate::http::handler::ApiHandler for $handler {
            fn handle_request(
                &self,
                method: &str,
                _path: &str,
                _body: &[u8],
                $params: &std::collections::HashMap<String, String>,
            ) -> $crate::http::types::HttpResponse {
                let $port = &self.state;
                let $adapter = match $crate::http::handlers::common::get_adapter_id(
                    method,
                    $port.adapter_count(),
                    $params,
                ) {
                    Ok(id) => id,
                    Err(error) => return error,
                };
                $body
            }
        }
    };
    (
        $(#[$meta:meta])*
        $handler:ident,
        state: $state:ident,
        id: $parse_id:path,
        respond: $respond:expr,
    ) => {
        declare_read_handler!(@shell $(#[$meta])* $handler, $state);
        declare_read_handler!(@get $handler, |state, adapter_id, params| {
            let resource_id = match $parse_id(params) {
                Ok(resource_id) => resource_id,
                Err(error) => return error,
            };
            ($respond)(state, adapter_id, resource_id)
        });
    };
    (
        $(#[$meta:meta])*
        $handler:ident,
        state: $state:ident,
        respond: $respond:expr,
    ) => {
        declare_read_handler!(@shell $(#[$meta])* $handler, $state);
        declare_read_handler!(@get $handler, |state, adapter_id, params| {
            ($respond)(state, adapter_id)
        });
    };
}

macro_rules! declare_metadata_patch_handler {
    (
        $(#[$meta:meta])*
        $handler:ident,
        state: $state:ident,
        watch: $watch:ident,
        watch_load: $watch_load:ident,
        data: $data:ident,
        id: $parse_id:path,
        parse: $parse:path,
        command: $command:ident { $id_field:ident, $ha_field:ident },
        origin: $origin:expr,
        echo: $echo:expr,
    ) => {
        pub struct $data {
            patch_mask: u8,
            name: Option<String>,
            ha_flag: Option<bool>,
        }

        $(#[$meta])*
        pub struct $handler {
            publisher: dali2rust_bus::BusPublisher,
            slots: std::sync::Arc<$crate::confirmation_bridge::PendingConfirmationSlots>,
            correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
            state: std::sync::Arc<dyn $state>,
            apply_watch: std::sync::Arc<dyn $watch>,
            bus_id: dali2rust_bus::BusId,
            timeout_ms: u64,
        }

        impl $handler {
            pub fn new(
                publisher: dali2rust_bus::BusPublisher,
                slots: std::sync::Arc<$crate::confirmation_bridge::PendingConfirmationSlots>,
                correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
                state: std::sync::Arc<dyn $state>,
                apply_watch: std::sync::Arc<dyn $watch>,
                bus_id: dali2rust_bus::BusId,
                timeout_ms: u64,
            ) -> Self {
                Self {
                    publisher,
                    slots,
                    correlation,
                    state,
                    apply_watch,
                    bus_id,
                    timeout_ms,
                }
            }
        }

        impl $crate::http::handlers::common::MutatingHandler for $handler {
            type Validated = (u8, u8, $data);
            type Executed = (u8, u8);

            fn expected_method(&self) -> &'static str {
                "PATCH"
            }

            fn validate(
                &self,
                params: &std::collections::HashMap<String, String>,
                body: &[u8],
            ) -> Result<(u8, u8, $data), $crate::http::types::HttpResponse> {
                let adapter_id = $crate::http::handlers::common::parse_adapter_id(
                    self.state.adapter_count(),
                    params,
                )?;
                let resource_id = $parse_id(params)?;
                let value = $crate::http::handlers::common::parse_json_body(body)?;
                let object = value.as_object().ok_or_else(|| {
                    $crate::http::handlers::common::json_err(400, "invalid_json")
                })?;
                let data = $parse(object)?;
                if data.patch_mask == 0 {
                    return Err($crate::http::handlers::common::json_err(400, "invalid_json"));
                }
                Ok((adapter_id, resource_id, data))
            }

            fn execute(
                &self,
                args: (u8, u8, $data),
            ) -> Result<(u8, u8), $crate::http::types::HttpResponse> {
                let (adapter_id, resource_id, data) = args;
                let correlation_id = self.correlation.next_id();
                let command = dali2rust_contracts::bus::command_envelope(
                    $crate::bus_codec::SOURCE_ID_UNSPECIFIED,
                    correlation_id,
                    self.bus_id.0,
                    $origin,
                    dali2rust_contracts::msg::$command {
                        adapter_id,
                        $id_field: resource_id,
                        patch_mask: data.patch_mask,
                        name: dali2rust_contracts::msg::fixed_text_64(
                            data.name.as_deref().unwrap_or(""),
                        ),
                        $ha_field: data.ha_flag.unwrap_or(false),
                    },
                );
                let watch = std::sync::Arc::clone(&self.apply_watch);
                $crate::http::handlers::common::publish_and_await_apply(
                    &self.publisher,
                    &self.slots,
                    correlation_id,
                    self.timeout_ms,
                    dali2rust_bus::BusFrame::command(command),
                    move || watch.$watch_load(),
                )?;
                Ok((adapter_id, resource_id))
            }

            fn respond(&self, args: (u8, u8)) -> $crate::http::types::HttpResponse {
                ($echo)(&self.state, args.0, args.1)
            }
        }
    };
}

macro_rules! declare_matrix_write_handler {
    (
        $(#[$meta:meta])*
        $handler:ident,
        state: $state:ident,
    ) => {
        $(#[$meta])*
        pub struct $handler {
            mode: $crate::http::handlers::common::MatrixWriteMode,
            publisher: dali2rust_bus::BusPublisher,
            correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
            state: std::sync::Arc<dyn $state>,
            bus_id: dali2rust_bus::BusId,
        }

        impl $handler {
            pub fn patch(
                publisher: dali2rust_bus::BusPublisher,
                correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
                state: std::sync::Arc<dyn $state>,
                bus_id: dali2rust_bus::BusId,
            ) -> Self {
                Self::with_mode(
                    $crate::http::handlers::common::MatrixWriteMode::Patch,
                    publisher,
                    correlation,
                    state,
                    bus_id,
                )
            }

            pub fn put(
                publisher: dali2rust_bus::BusPublisher,
                correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
                state: std::sync::Arc<dyn $state>,
                bus_id: dali2rust_bus::BusId,
            ) -> Self {
                Self::with_mode(
                    $crate::http::handlers::common::MatrixWriteMode::Replace,
                    publisher,
                    correlation,
                    state,
                    bus_id,
                )
            }

            fn with_mode(
                mode: $crate::http::handlers::common::MatrixWriteMode,
                publisher: dali2rust_bus::BusPublisher,
                correlation: std::sync::Arc<$crate::http::dispatcher::CorrelationIdAllocator>,
                state: std::sync::Arc<dyn $state>,
                bus_id: dali2rust_bus::BusId,
            ) -> Self {
                Self {
                    mode,
                    publisher,
                    correlation,
                    state,
                    bus_id,
                }
            }
        }
    };
}

macro_rules! declare_handler_shell {
    (
        $(#[$meta:meta])*
        $handler:ident {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $handler {
            $( $(#[$fmeta])* $field: $ty, )+
        }

        impl $handler {
            #[allow(clippy::too_many_arguments, reason = "One construction site each — composition — and the parameter order is the FIELD order, so a shell wired through `app_router::mutation(...)` must declare its fields in `MutationCtor` order")]
            pub fn new( $( $field: $ty, )+ ) -> Self {
                Self { $( $field, )+ }
            }
        }
    };
}

pub(crate) use declare_handler_shell;
pub(crate) use declare_matrix_write_handler;
pub(crate) use declare_metadata_patch_handler;
pub(crate) use declare_read_handler;
