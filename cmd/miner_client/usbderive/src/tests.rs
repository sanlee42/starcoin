#[cfg(test)]
mod tests {
    use crate::derive::{Config, UsbDerive};
    use crate::proto::{Message, DeriveResponse};
    use crate::read_until;
    use anyhow::{Result, Error};
    use byteorder::{LittleEndian, WriteBytesExt};
    use std::{io, thread};
    use std::time::Duration;
    use futures::channel::mpsc::unbounded;
    use std::thread::spawn;
    use futures::executor::block_on;
    use futures::StreamExt;
    use starcoin_consensus::{set_header_nonce, Consensus};
    use std::convert::TryInto;

    const INPUT_DATA: [u8; 76] = [
        0x05, 0x05, 0xc0, 0xa7, 0xdb, 0xc7, 0x05, 0xb0, 0xad, 0xf8, 0x2c, 0x58, 0x1a, 0xae, 0xe4,
        0x8b, 0x2e, 0x0a, 0xee, 0x2e, 0xa8, 0x97, 0x2d, 0xd7, 0x9d, 0xba, 0xf3, 0xca, 0x28, 0xac,
        0xca, 0x5f, 0x73, 0xca, 0x2a, 0x90, 0x9c, 0x8c, 0x24, 0xf7, 0x09, 0x00, 0x80, 0xf9, 0x87,
        0x13, 0xc6, 0x91, 0x9a, 0x42, 0x38, 0x9d, 0x53, 0xcb, 0xde, 0xd0, 0x4d, 0x02, 0x6c, 0x1d,
        0xe4, 0x25, 0xf8, 0x77, 0xe8, 0x70, 0xb3, 0x8f, 0x91, 0x4c, 0xef, 0x40, 0xc6, 0x7f, 0xa4,
        0x00,
    ];

    #[test]
    fn test_caculate_hash() {
        let nonce: u32 = 0x66;
        let hash: [u8; 32] = [
            0, 206, 21, 48, 113, 70, 23, 142, 165, 134, 28, 6, 134, 104, 109, 247, 66, 43, 91, 44,
            182, 222, 221, 157, 206, 0, 101, 61, 152, 172, 204, 141,
        ];
        let mut input = INPUT_DATA.clone();
        let mut nonce_b = vec![];
        nonce_b.write_u32::<LittleEndian>(nonce).unwrap();
        input[39] = nonce_b[0];
        input[40] = nonce_b[1];
        input[41] = nonce_b[2];
        input[42] = nonce_b[3];
        for i in input.iter() {
            print!("{:?},", i);
        }
        println!("");
        let hash = starcoin_consensus::CRYPTONIGHT.calculate_pow_hash(&INPUT_DATA, nonce as u64).unwrap();
        println!("{:?}", &hash.to_vec());
    }

    fn setup(path: &str) -> Result<UsbDerive> {
        let mut derive = UsbDerive::open(path, Config::default()).expect("Must open serial port");
        derive.set_hw_params();
        derive.set_opcode()?;
        derive.get_state()?;
        Ok(derive)
    }

    #[test]
    fn test_derive_set() {
        let path = "/dev/cu.usbmodem2065325550561";
        let mut derive = setup(path).unwrap();

        let (tx, mut rx) = unbounded();
        let mut derive_clone = derive.clone();
        thread::spawn(move || { derive_clone.process_seal(tx); });
        derive.set_job(0x1, 0x006fffff, &INPUT_DATA).unwrap();
        block_on(async {
            loop {
                let d = rx.next().await.unwrap();
                println!("{:?},{}", d.0, d.1);
            }
        });
    }

    #[test]
    fn test_check_hash() {
        let expect_hash: [u8; 32] = [0, 13, 253, 164, 33, 127, 6, 170, 87, 93, 38, 89, 96, 239, 174, 122, 1, 226, 105, 209, 163, 81, 205, 222, 250, 36, 170, 84, 24, 158, 242, 162];
        let nonce: u64 = 637;

        let hash = starcoin_consensus::CRYPTONIGHT.calculate_pow_hash(&INPUT_DATA, nonce).unwrap();
        let h: [u8; 32] = hash.to_vec().as_slice().try_into().unwrap();
        assert_eq!(expect_hash, h);
    }
}