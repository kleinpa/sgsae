pub type Register = usize;
pub type Address = usize;
pub type ProgramCounter = Address;

pub trait Memory {
    fn load(&self, a: Address) -> u8;
    fn store(&mut self, a: Address, x: u8);
    fn store_slice(&mut self, a: Address, xs: &[u8]) {
        for (i, x) in xs.iter().enumerate() {
            self.store(a + i, *x);
        }
    }
    fn load_array<const N: usize>(&mut self, a: Address) -> [u8; N] {
        let mut xs = [0u8; N];
        for i in 0..N {
            xs[i] = self.load(a + i);
        }
        xs
    }
}
