pub mod dali;
pub mod health;

pub trait BufferBytes {
    fn inner_data(&self) -> &Vec<u8>;

    fn as_bytes(&self) -> &[u8] {
        self.inner_data().as_slice()
    }

    fn to_vec(&self) -> Vec<u8> {
        self.inner_data().clone()
    }
}
