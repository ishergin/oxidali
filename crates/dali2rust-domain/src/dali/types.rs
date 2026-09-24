pub use crate::dali::net::address::decode_wire_address;
pub use crate::dali::net::address::{DaliAddress, DaliAddressError};

macro_rules! dali_index {
    ($name:ident, $max:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name(u8);

        impl $name {
            pub fn new(index: u8) -> Result<Self, DaliAddressError> {
                if index > $max {
                    return Err(DaliAddressError::OutOfRange {
                        max: $max,
                        got: index,
                    });
                }
                Ok(Self(index))
            }

            pub fn index(&self) -> u8 {
                self.0
            }
        }
    };
}

dali_index!(SceneIndex, 15);
dali_index!(GroupIndex, 15);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_index_valid() {
        assert_eq!(SceneIndex::new(0).unwrap().index(), 0);
        assert_eq!(SceneIndex::new(15).unwrap().index(), 15);
    }

    #[test]
    fn scene_index_out_of_range() {
        assert_eq!(
            SceneIndex::new(16),
            Err(DaliAddressError::OutOfRange { max: 15, got: 16 })
        );
    }

    #[test]
    fn group_index_valid() {
        assert_eq!(GroupIndex::new(0).unwrap().index(), 0);
        assert_eq!(GroupIndex::new(15).unwrap().index(), 15);
    }
}
