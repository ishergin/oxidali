pub trait BitbangHal {
    fn bus_is_high(&mut self) -> bool;
    fn bus_set_low(&mut self);
    fn bus_set_high(&mut self);
}
