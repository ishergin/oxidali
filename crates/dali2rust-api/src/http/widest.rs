macro_rules! declare_widest_dtos {
    ( $( $ty:ident { $( $field:ident $( = $widest:expr )? ),* $(,)? } )* ) => {
        $(
            impl $ty {
                #[cfg(test)]
                pub(crate) fn widest() -> Self {
                    Self { $( $field: declare_widest_dtos!(@widest $($widest)?), )* }
                }
            }
        )*
    };
    (@widest $widest:expr) => { $widest };
    (@widest) => { u32::MAX };
}
