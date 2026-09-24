macro_rules! standard_http_method_wire {
    ($m:expr, $($variant:path => $wire:literal),+ $(,)?) => {
        match $m {
            $($variant => $wire,)+
            other => {
                log::warn!("unknown HTTP method: {other:?}");
                "OTHER"
            }
        }
    };
}

#[cfg(not(target_os = "espidf"))]
use tiny_http::Method as TinyMethod;

#[cfg(not(target_os = "espidf"))]
pub fn wire_method(m: &TinyMethod) -> &'static str {
    standard_http_method_wire!(
        m,
        TinyMethod::Get => "GET",
        TinyMethod::Post => "POST",
        TinyMethod::Put => "PUT",
        TinyMethod::Delete => "DELETE",
        TinyMethod::Patch => "PATCH",
    )
}

#[cfg(target_os = "espidf")]
use esp_idf_svc::http::server::Method as EspMethod;

#[cfg(target_os = "espidf")]
pub fn wire_method(m: EspMethod) -> &'static str {
    standard_http_method_wire!(
        m,
        EspMethod::Get => "GET",
        EspMethod::Post => "POST",
        EspMethod::Put => "PUT",
        EspMethod::Delete => "DELETE",
        EspMethod::Patch => "PATCH",
    )
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "espidf"))]
    #[test]
    fn wire_method_conversions() {
        use super::*;
        use tiny_http::Method as TinyMethod;

        assert_eq!(wire_method(&TinyMethod::Get), "GET");
        assert_eq!(wire_method(&TinyMethod::Post), "POST");
        assert_eq!(wire_method(&TinyMethod::Put), "PUT");
        assert_eq!(wire_method(&TinyMethod::Delete), "DELETE");
        assert_eq!(wire_method(&TinyMethod::Patch), "PATCH");
    }
}
