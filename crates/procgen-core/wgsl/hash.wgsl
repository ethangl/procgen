// WGSL mirror of procgen-core's four-word 32-bit hash.
// Keep expression order aligned with hash32.rs.

fn hash_u32(word0: u32, word1: u32, word2: u32, word3: u32) -> u32 {
    var value = (word0 + 0x9e3779b9u)
        ^ (word1 * 0x85ebca6bu)
        ^ (word2 * 0xc2b2ae35u)
        ^ (word3 * 0x27d4eb2fu);
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    return value ^ (value >> 16u);
}
