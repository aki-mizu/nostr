// Copyright (c) 2022 Yuki Kishimoto
// Distributed under the MIT software license

extern crate nostr;

use std::error::Error;

use nostr::key::{FromMnemonic, GenerateMnemonic, Keys, ToBech32};

const MNEMONIC_PHRASE: &str = "equal dragon fabric refuse stable cherry smoke allow alley easy never medal attend together lumber movie what sad siege weather matrix buffalo state shoot";

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    println!("Mnemonic: {}", Keys::generate_mnemonic(12)?);

    let keys = Keys::from_mnemonic(MNEMONIC_PHRASE)?;
    println!("{}", keys.secret_key()?.to_bech32()?);

    Ok(())
}
