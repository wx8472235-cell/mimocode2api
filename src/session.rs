const ALPHA: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn gen_session_affinity(now_ms: i64) -> String {
    let y = now_ms * 4096 + 1;
    let low48 = (!y as u64) & 0xFFFF_FFFF_FFFF;

    let mut rand_bytes = [0u8; 14];
    getrandom::fill(&mut rand_bytes).expect("getrandom 失败");

    let mut s = format!("ses_{low48:012x}");
    s.extend(rand_bytes.iter().map(|b| ALPHA[(*b as usize) % 62] as char));
    s
}

#[cfg(test)]
#[path = "../tests/session.rs"]
mod tests;
