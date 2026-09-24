#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    pub fn next_24(&mut self) -> u32 {
        self.next_u32() & 0x00FF_FFFF
    }

    pub fn chance_permille(&mut self, permille: u16) -> bool {
        if permille == 0 {
            return false;
        }
        u32::from(permille) > self.next_u32() % 1000
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn zero_seed_still_produces_a_sequence() {
        let mut rng = Rng::new(0);
        let a = rng.next_u32();
        let b = rng.next_u32();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn next_24_stays_in_range_and_varies() {
        let mut rng = Rng::new(1);
        let values: Vec<u32> = (0..64).map(|_| rng.next_24()).collect();
        assert!(values.iter().all(|v| *v <= 0x00FF_FFFF));
        assert!(values.iter().any(|v| *v != values[0]));
    }

    #[test]
    fn same_seed_replays_the_same_run() {
        let mut a = Rng::new(0xBEEF);
        let mut b = Rng::new(0xBEEF);
        assert_eq!(a.next_24(), b.next_24());
    }

    #[test]
    fn zero_permille_never_fires_and_thousand_always_does() {
        let mut rng = Rng::new(7);
        assert!(!rng.chance_permille(0));
        assert!(rng.chance_permille(1000));
    }
}
