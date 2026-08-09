pub fn fnv1a32(data: &[u8]) -> u32 {
    if data.len() == 0 {
        return 0;
    }
    const OFFSET_BASIS: u32 = 0x811c9dc5;
    const FNV_PRIME: u32 = 0x01000193;
    let mut hash: u32 = OFFSET_BASIS;
    for &b in data {
        hash ^= b as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as u32
}
