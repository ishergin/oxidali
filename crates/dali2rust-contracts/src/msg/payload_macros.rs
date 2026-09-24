macro_rules! __payload_budget_max {
    () => {
        crate::bus::MAX_BUS_WIRE_BYTES
    };
    ($max:expr) => {
        $max
    };
}
#[cfg(test)]
pub(crate) use __payload_budget_max;

pub(crate) const fn name_position(names: &[&str], name: &str) -> usize {
    let mut i = 0;
    while i < names.len() {
        if const_str_eq(names[i], name) {
            return i;
        }
        i += 1;
    }
    panic!("variant name missing from the names const")
}

const fn const_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

macro_rules! declare_bus_payloads {
    (
        $(#[$umeta:meta])*
        union $union:ident;
        names $names:ident;
        tests $testmod:ident;
        probe $probe:path;
        extern {
            $(
                $ext:ident { budget = $ext_sample:expr $(, max = $ext_max:expr)? ; }
            )+
        }
        $(
            $(#[$smeta:meta])*
            pub struct $name:ident {
                $(
                    $(#[$fmeta:meta])*
                    pub $field:ident : $fty:ty
                ),* $(,)?
            }
            budget = $sample:expr $(, max = $max:expr)? ;
        )+
    ) => {
        $(
            $(#[$smeta])*
            #[derive(Clone, Debug, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
            pub struct $name {
                $(
                    $(#[$fmeta])*
                    pub $field: $fty,
                )*
            }
        )+

        $(#[$umeta])*
        #[derive(Clone, Debug, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
        pub enum $union {
            $( $ext($ext), )+
            $( $name($name), )+
        }

        $(
            impl ::core::convert::From<$ext> for $union {
                fn from(v: $ext) -> Self {
                    Self::$ext(v)
                }
            }
        )+
        $(
            impl ::core::convert::From<$name> for $union {
                fn from(v: $name) -> Self {
                    Self::$name(v)
                }
            }
        )+

        pub const $names: &[&str] = &[
            $( stringify!($ext), )+
            $( stringify!($name), )+
        ];

        impl $union {
            pub fn variant_name(&self) -> &'static str {
                match self {
                    $( Self::$ext(_) => stringify!($ext), )+
                    $( Self::$name(_) => stringify!($name), )+
                }
            }

            pub fn variant_index(&self) -> usize {
                match self {
                    $( Self::$ext(_) => const { crate::msg::payload_macros::name_position($names, stringify!($ext)) }, )+
                    $( Self::$name(_) => const { crate::msg::payload_macros::name_position($names, stringify!($name)) }, )+
                }
            }
        }

        #[cfg(test)]
        mod $testmod {
            use super::*;

            fn samples() -> impl Iterator<Item = (&'static str, $union, usize)> {
                [
                    $(
                        (
                            stringify!($ext),
                            $union::from($ext_sample),
                            crate::msg::payload_macros::__payload_budget_max!($($ext_max)?),
                        ),
                    )+
                    $(
                        (
                            stringify!($name),
                            $union::from($sample),
                            crate::msg::payload_macros::__payload_budget_max!($($max)?),
                        ),
                    )+
                ]
                .into_iter()
            }

            #[test]
            fn postcard_budget_per_variant() {
                for (name, payload, max) in samples() {
                    let len = $probe(&payload);
                    assert!(
                        len <= max,
                        "{name}: worst-case envelope {len} B exceeds budget {max} B"
                    );
                }
            }

            #[test]
            fn postcard_discriminant_matches_declaration_order() {
                for (i, (name, payload, _)) in samples().enumerate() {
                    let bytes = ::postcard::to_allocvec(&payload).expect(name);
                    assert!(i < 128, "{name}: index {i} breaks 1-byte varint assumption");
                    assert_eq!(
                        bytes[0] as usize, i,
                        "{name}: postcard wire discriminant moved (declaration order is a wire contract)"
                    );
                }
            }

            #[test]
            fn postcard_roundtrip_per_variant() {
                for (name, payload, _) in samples() {
                    let bytes = ::postcard::to_allocvec(&payload).expect(name);
                    let back: $union = ::postcard::from_bytes(&bytes).expect(name);
                    assert_eq!(back, payload, "{name}: postcard roundtrip mismatch");
                }
            }

            #[test]
            fn variant_identity_matches_declaration_order() {
                for (i, (name, payload, _)) in samples().enumerate() {
                    assert_eq!(payload.variant_index(), i, "{name}: variant_index drifted");
                    assert_eq!(payload.variant_name(), name, "{name}: variant_name drifted");
                }
            }
        }
    };
}
pub(crate) use declare_bus_payloads;

#[macro_export]
macro_rules! dispatch_bus_payload {
    (
        union = $union:ident;
        $(#[$cmeta:meta])*
        pub const $handled:ident;
        $(#[$fnmeta:meta])*
        $vis:vis fn $fname:ident $([$($gen:tt)*])? ( $($params:tt)* );
        payload = $payload:expr;
        ignored = $ignored:block;
        $(
            $variant:ident($pat:tt) => $body:expr
        ),+ $(,)?
    ) => {
        $(#[$cmeta])*
        pub const $handled: &[&str] = &[ $( stringify!($variant) ),+ ];

        $(#[$fnmeta])*
        $vis fn $fname $(<$($gen)*>)? ( $($params)* ) {
            match $payload {
                $(
                    $crate::msg::$union::$variant($pat) => { $body }
                )+
                #[allow(unreachable_patterns, reason = "A dispatch table covering every union variant makes the trailing `_` unreachable; it is kept so partial tables (a worker handling a subset) still compile")]
                _ => $ignored,
            }
        }
    };
}

#[macro_export]
macro_rules! dispatch_bus_commands {
    ($($t:tt)*) => {
        $crate::dispatch_bus_payload! { union = BusCommandPayload; $($t)* }
    };
}

#[macro_export]
macro_rules! dispatch_bus_events {
    ($($t:tt)*) => {
        $crate::dispatch_bus_payload! { union = BusEventPayload; $($t)* }
    };
}
