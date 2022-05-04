use crate::types::Address;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum MemoryError {
    InvalidAddress(u64),
    ReadOnlyAddress(u64),
}

type Result<T> = std::result::Result<T, MemoryError>;

/// A generalized load/store memory interface
pub trait Memory<T: Default + Copy> {
    fn load(&self, a: Address) -> Result<T>;
    fn store(&mut self, a: Address, x: T) -> Result<()>;
    fn store_slice(&mut self, a: Address, xs: &[T]) -> Result<()> {
        for (i, x) in xs.iter().enumerate() {
            self.store(a.wrapping_add(i.try_into().unwrap()), *x)?;
        }
        Ok(())
    }
    fn load_array<const N: usize>(&self, a: Address) -> Result<[T; N]> {
        let mut arr: [T; N] = [T::default(); N];
        for (i, item) in arr.iter_mut().enumerate() {
            *item = self.load(a.wrapping_add(i as u64)).unwrap()
        }
        Ok(arr)
    }
}

#[derive(Debug)]
enum MemoryReference<'a> {
    RW(&'a mut [u8]),
    RO(&'a [u8]),
}

#[derive(Debug)]
struct MemoryMapEntry<'a> {
    start: u64,
    length: u64,
    data: MemoryReference<'a>,
}

#[derive(Debug)]
pub struct MemoryMap<'a> {
    map: Vec<MemoryMapEntry<'a>>,
}
impl<'a> MemoryMap<'a> {
    pub fn new() -> MemoryMap<'static> {
        MemoryMap { map: Vec::new() }
    }
    pub fn map_ro(&mut self, start: u64, data: &'a [u8]) {
        self.map.push(MemoryMapEntry {
            start,
            length: data.len() as u64,
            data: MemoryReference::RO(data),
        })
    }
    pub fn map(&mut self, start: u64, data: &'a mut [u8]) {
        self.map.push(MemoryMapEntry {
            start,
            length: data.len() as u64,
            data: MemoryReference::RW(data),
        })
    }
}
impl<'a> Memory<u8> for MemoryMap<'a> {
    fn load(&self, a: crate::types::Address) -> std::result::Result<u8, MemoryError> {
        for entry in self.map.iter().rev() {
            if a >= entry.start && a < (entry.start + entry.length) {
                return match &entry.data {
                    MemoryReference::RW(data) => Ok(data[(a - entry.start) as usize]),
                    MemoryReference::RO(data) => Ok(data[(a - entry.start) as usize]),
                };
            }
        }
        return Err(MemoryError::InvalidAddress(a));
    }

    fn store(&mut self, a: crate::types::Address, x: u8) -> std::result::Result<(), MemoryError> {
        for entry in self.map.iter_mut().rev() {
            if a >= entry.start && a < (entry.start + entry.length) {
                return match &mut entry.data {
                    MemoryReference::RW(data) => {
                        data[(a - entry.start) as usize] = x;
                        return Ok(());
                    }
                    MemoryReference::RO(_) => Err(MemoryError::ReadOnlyAddress(a)),
                };
            }
        }
        return Err(MemoryError::InvalidAddress(a));
    }
}

#[cfg(test)]
mod tests {
    use crate::memory::Memory;

    use super::MemoryMap;

    #[test]
    fn test_memory() {
        let mut d1: [u8; 8] = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7];
        let d2: [u8; 8] = [0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7];
        {
            let mut mem = MemoryMap::new();
            mem.map(0, &mut d1);
            mem.map_ro(4, &d2);

            assert_eq!(mem.load(0).unwrap(), 0xA0);
            assert_eq!(mem.load(4).unwrap(), 0xB0);
            assert_eq!(mem.load(11).unwrap(), 0xB7);

            assert!(mem.store(0, 0xC0).is_ok());
            assert!(mem.store(4, 0xD0).is_err());
            assert!(mem.store(14, 0xD0).is_err());
        }
        assert_eq!(d1[0], 0xC0);
        assert_eq!(d2[0], 0xB0);
    }
}
