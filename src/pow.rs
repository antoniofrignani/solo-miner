use anyhow::{Context, Result, ensure};
use bitcoin::hashes::{Hash, sha256d};
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive};
use serde_json::Value;

pub fn hash(header: &[u8]) -> [u8; 32] {
    sha256d::Hash::hash(header).to_byte_array()
}

pub fn meets(hash: &[u8; 32], target: &[u8; 32]) -> bool {
    hash.iter().rev().cmp(target.iter().rev()).is_le()
}

fn bytes(value: BigUint) -> Result<[u8; 32]> {
    let raw = value.to_bytes_le();
    ensure!(raw.len() <= 32, "Target oltre 256 bit");
    let mut out = [0; 32];
    out[..raw.len()].copy_from_slice(&raw);
    Ok(out)
}

pub fn compact_target(bits: u32) -> Result<[u8; 32]> {
    let exponent = bits >> 24;
    let mantissa = bits & 0x007fffff;
    ensure!(
        bits & 0x00800000 == 0 && mantissa != 0 && exponent <= 32,
        "nBits non valido"
    );
    let value = BigUint::from(mantissa);
    bytes(if exponent <= 3 {
        value >> (8 * (3 - exponent))
    } else {
        value << (8 * (exponent - 3))
    })
}

pub fn difficulty(target: &[u8; 32]) -> f64 {
    let one = BigUint::from_bytes_le(&compact_target(0x1d00ffff).expect("diff1"));
    one.to_f64().unwrap() / BigUint::from_bytes_le(target).to_f64().unwrap()
}

/// Exact rational conversion of the server's IEEE754 difficulty, with integer floor.
pub fn share_target(diff: f64) -> Result<[u8; 32]> {
    ensure!(
        diff.is_finite() && diff > 0.0,
        "Difficoltà share non valida"
    );
    let bits = diff.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, shift) = if exp == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), exp - 1023 - 52)
    };
    let one = BigUint::from_bytes_le(&compact_target(0x1d00ffff)?);
    let value = if shift >= 0 {
        one / (BigUint::from(mantissa) << shift as usize)
    } else {
        (one << (-shift) as usize) / BigUint::from(mantissa)
    };
    bytes(value.min((BigUint::one() << 256usize) - BigUint::one()))
}

#[derive(Clone)]
pub struct Job {
    pub id: String,
    pub ntime: String,
    pub header: [u8; 80],
    pub network_target: [u8; 32],
    pub share_target: [u8; 32],
    pub share_difficulty: f64,
    pub extranonce2: String,
}

pub fn build_job(params: &Value, extra1: &[u8], extra2: &[u8], diff: f64) -> Result<Job> {
    let p = params.as_array().context("Job non valido")?;
    ensure!(
        p.len() == 9 && p[8].is_boolean(),
        "Parametri job non validi"
    );
    let text = |i: usize| p[i].as_str().context("Campo job non testuale");
    let id = text(0)?.to_owned();
    ensure!(id.len() <= 256, "Job ID troppo lungo");
    let mut coinbase = hex::decode(text(2)?)?;
    coinbase.extend_from_slice(extra1);
    coinbase.extend_from_slice(extra2);
    coinbase.extend(hex::decode(text(3)?)?);
    let mut merkle = hash(&coinbase);
    let branches = p[4].as_array().context("Merkle branches non valide")?;
    ensure!(branches.len() <= 32, "Troppi merkle branches");
    for branch in branches {
        let branch = hex::decode(branch.as_str().context("Merkle branch non valido")?)?;
        ensure!(branch.len() == 32, "Lunghezza merkle branch errata");
        let mut pair = Vec::from(merkle);
        pair.extend(branch);
        merkle = hash(&pair);
    }
    let mut header = [0; 80];
    let field = |i: usize| -> Result<u32> {
        ensure!(text(i)?.len() == 8, "Campo header deve essere di 4 byte");
        Ok(u32::from_str_radix(text(i)?, 16)?)
    };
    header[..4].copy_from_slice(&field(5)?.to_le_bytes());
    let mut prev = hex::decode(text(1)?)?;
    ensure!(prev.len() == 32, "Prevhash deve essere di 32 byte");
    // Stratum V1 previous hash uses big-endian words; header serialization uses LE words.
    for word in prev.as_chunks_mut::<4>().0 {
        word.reverse();
    }
    header[4..36].copy_from_slice(&prev);
    header[36..68].copy_from_slice(&merkle);
    header[68..72].copy_from_slice(&field(7)?.to_le_bytes());
    let bits = field(6)?;
    header[72..76].copy_from_slice(&bits.to_le_bytes());
    Ok(Job {
        id,
        ntime: text(7)?.to_owned(),
        header,
        network_target: compact_target(bits)?,
        share_target: share_target(diff)?,
        share_difficulty: diff,
        extranonce2: hex::encode(extra2),
    })
}

pub fn genesis_header() -> [u8; 80] {
    hex::decode(concat!(
        "01000000",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
        "29ab5f49",
        "ffff001d",
        "1dac2b7c"
    ))
    .unwrap()
    .try_into()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genesis_pow_and_targets() {
        let h = hash(&genesis_header());
        assert_eq!(
            hex::encode(h),
            "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000"
        );
        assert!(meets(&h, &compact_target(0x1d00ffff).unwrap()));
        assert_eq!(
            share_target(1.0).unwrap(),
            compact_target(0x1d00ffff).unwrap()
        );
        assert_eq!(difficulty(&share_target(512.0).unwrap()), 512.0);
        assert!(share_target(0.0).is_err());
        assert!(share_target(f64::NAN).is_err());
        assert!(compact_target(0x1d80ffff).is_err());
    }
    #[test]
    fn constructs_genesis_from_canonical_coinbase() {
        let block = bitcoin::blockdata::constants::genesis_block(bitcoin::Network::Bitcoin);
        let coinbase = bitcoin::consensus::serialize(&block.txdata[0]);
        let params = serde_json::json!([
            "genesis",
            "00".repeat(32),
            hex::encode(coinbase),
            "",
            [],
            "00000001",
            "1d00ffff",
            "495fab29",
            true
        ]);
        let mut job = build_job(&params, &[], &[], 1.0).unwrap();
        job.header[76..80].copy_from_slice(&block.header.nonce.to_le_bytes());
        assert_eq!(
            job.header.as_slice(),
            bitcoin::consensus::serialize(&block.header)
        );
    }
    #[test]
    fn job_serialization_and_merkle() {
        let prev: Vec<u8> = (0..32).collect();
        let p = serde_json::json!([
            "job",
            hex::encode(&prev),
            "0102",
            "0304",
            [],
            "20000000",
            "1d00ffff",
            "495fab29",
            true
        ]);
        let job = build_job(&p, &[5], &[6, 7], 2.0).unwrap();
        assert_eq!(&job.header[..4], &[0, 0, 0, 32]);
        assert_eq!(&job.header[4..8], &[3, 2, 1, 0]);
        assert_eq!(&job.header[36..68], &hash(&[1, 2, 5, 6, 7, 3, 4]));
        assert_eq!(
            &job.header[68..76],
            &[0x29, 0xab, 0x5f, 0x49, 0xff, 0xff, 0, 0x1d]
        );
        assert_eq!(job.extranonce2, "0607");
    }
}
