use core::ops::Deref;

pub(crate) struct BitsBase<T>(T)
where
    T: AsRef<[u8]>;

impl<T> BitsBase<T>
where
    T: AsRef<[u8]>,
{
    pub(crate) fn get_bit(&self, idx: usize) -> u8 {
        self.0.as_ref()[idx / 8] >> (7 - (idx % 8)) & 0x01
    }

    pub(crate) fn iter(&self) -> BitsIterator<T> {
        BitsIterator { bits: self, idx: 0 }
    }
}

pub(crate) struct BitsIterator<'a, T>
where
    T: AsRef<[u8]>,
{
    bits: &'a BitsBase<T>,
    idx: usize,
}

impl<T> Iterator for BitsIterator<'_, T>
where
    T: AsRef<[u8]>,
{
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.bits.0.as_ref().len() * 8 {
            return None;
        }
        let bit = self.bits.get_bit(self.idx);
        self.idx += 1;
        Some(bit)
    }
}

pub(crate) struct Bits<'a>(BitsBase<&'a [u8]>);

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self(BitsBase(data))
    }
}

impl<'a> Deref for Bits<'a> {
    type Target = BitsBase<&'a [u8]>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct BitsMut<'a>(BitsBase<&'a mut [u8]>);

impl<'a> Deref for BitsMut<'a> {
    type Target = BitsBase<&'a mut [u8]>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> BitsMut<'a> {
    pub(crate) fn new(data: &'a mut [u8]) -> Self {
        Self(BitsBase(data))
    }

    pub(crate) fn set_bit(&mut self, idx: usize, value: u8) {
        let slice = &mut self.0 .0;
        let existing = slice[idx / 8];
        if value == 0 {
            slice[idx / 8] = existing & !(1 << (7 - (idx % 8)));
        } else {
            slice[idx / 8] = existing | (1 << (7 - (idx % 8)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_readonly() {
        let data: [u8; 2] = [0b00001111, 0b10101010];
        let bits = Bits::new(&data);
        assert_eq!(bits.get_bit(0), 0);
        assert_eq!(bits.get_bit(1), 0);
        assert_eq!(bits.get_bit(4), 1);
        assert_eq!(bits.get_bit(8), 1);
        assert_eq!(bits.get_bit(9), 0);
    }

    #[test]
    fn bits_modifying() {
        let mut data: [u8; 2] = [0b00001111, 0b10101010];
        let mut bits = BitsMut::new(&mut data);

        assert_eq!(bits.get_bit(0), 0);
        bits.set_bit(0, 1);
        assert_eq!(bits.get_bit(0), 1);

        assert_eq!(bits.get_bit(4), 1);
        bits.set_bit(4, 0);
        assert_eq!(bits.get_bit(4), 0);

        assert_eq!(bits.get_bit(9), 0);
        bits.set_bit(9, 1);
        assert_eq!(bits.get_bit(9), 1);

        assert_eq!(data, [0b10000111, 0b11101010]);
    }

    #[test]
    fn bits_iter() {
        let data: [u8; 2] = [0b00110111, 0b10101010];
        let bits = Bits::new(&data);
        let mut it = bits.iter();
        assert_eq!(it.next(), Some(0));
        assert_eq!(it.next(), Some(0));
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(0));
        assert_eq!(it.next(), Some(1));
        for _ in 0..8 {
            let _ = it.next();
        }
        assert_eq!(it.next(), Some(1));
        assert_eq!(it.next(), Some(0));
        assert_eq!(it.next(), None);
    }
}
