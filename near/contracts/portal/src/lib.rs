//#![allow(unused_mut)]
//#![allow(unused_imports)]
//#![allow(unused_variables)]
//#![allow(dead_code)]

use near_contract_standards::fungible_token::metadata::{FungibleTokenMetadata, FT_METADATA_SPEC};
use near_sdk::json_types::{Base64VecU8, U128};

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, UnorderedSet};
use near_sdk::{
    env, ext_contract, near_bindgen, require, AccountId, Balance, Gas, PanicOnDefault, Promise,
    PromiseError, PromiseOrValue, PublicKey,
};
use serde::{Deserialize, Serialize};

use near_sdk::utils::is_promise_success;

use std::str;

pub mod byte_utils;
pub mod state;

use crate::byte_utils::{get_string_from_32, ByteUtils};

// near_sdk::setup_alloc!();

const CHAIN_ID_NEAR: u16 = 15;
const CHAIN_ID_SOL: u16 = 1;

const BRIDGE_TOKEN_BINARY: &[u8] =
    include_bytes!("../../ft/target/wasm32-unknown-unknown/release/ft.wasm");

/// Initial balance for the BridgeToken contract to cover storage and related.
const TRANSFER_BUFFER: u128 = 1000;

const NEAR_MULT: u128 = 10_000_000_000_000_000; // 1e16

/// Gas to initialize BridgeToken contract.
//const BRIDGE_TOKEN_NEW: Gas = Gas(100_000_000_000_000);

/// Gas to call mint method on bridge token.
//const MINT_GAS: Gas = Gas(10_000_000_000_000);

#[ext_contract(ext_ft_contract)]
pub trait FtContract {
    fn new(metadata: FungibleTokenMetadata, asset_meta: Vec<u8>, seq_number: u64) -> Self;
    fn update_ft(metadata: FungibleTokenMetadata, asset_meta: Vec<u8>, seq_number: u64);
    fn ft_transfer_call(
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128>;
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
    fn ft_metadata(&self) -> FungibleTokenMetadata;
    fn vaa_transfer(
        &self,
        amount: u128,
        token_address: Vec<u8>,
        token_chain: u16,
        account_id: AccountId,
        recipient_chain: u16,
        fee: u128,
    );
    fn vaa_withdraw(
        &self,
        from: AccountId,
        amount: u128,
        receiver: String,
        chain: u16,
        fee: u128,
        payload: String,
    ) -> String;
}

#[ext_contract(ext_worm_hole)]
pub trait Wormhole {
    fn verify_vaa(&self, vaa: String) -> u32;
    fn publish_message(&self, data: String, nonce: u32) -> u64;
}

#[ext_contract(ext_portal)]
pub trait ExtPortal {
    fn finish_deploy(&self, token: AccountId);
}

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct TransferMsgPayload {
    receiver: String,
    chain: u16,
    fee: String,
    payload: String,
    message_fee: Balance,
}

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct TokenData {
    meta: Vec<u8>,
    decimals: u8,

    address: String,
    chain: u16,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
pub struct Portal {
    booted: bool,
    core: AccountId,
    dups: UnorderedSet<Vec<u8>>,
    owner_pk: PublicKey,
    emitter_registration: LookupMap<u16, Vec<u8>>,
    last_asset: u32,
    upgrade_hash: Vec<u8>,

    tokens: LookupMap<AccountId, TokenData>,
    key_map: LookupMap<Vec<u8>, AccountId>,
    hash_map: LookupMap<Vec<u8>, AccountId>,

    bank: LookupMap<AccountId, Balance>,
}

impl Default for Portal {
    fn default() -> Self {
        Self {
            booted: false,
            core: AccountId::new_unchecked("".to_string()),
            dups: UnorderedSet::new(b"d".to_vec()),
            owner_pk: env::signer_account_pk(),
            emitter_registration: LookupMap::new(b"c".to_vec()),
            last_asset: 0,
            upgrade_hash: b"".to_vec(),

            tokens: LookupMap::new(b"t".to_vec()),
            key_map: LookupMap::new(b"k".to_vec()),
            hash_map: LookupMap::new(b"a".to_vec()),

            bank: LookupMap::new(b"b".to_vec()),
        }
    }
}

fn vaa_register_chain(
    storage: &mut Portal,
    vaa: &state::ParsedVAA,
    mut deposit: Balance,
    refund_to: &AccountId,
) -> Balance {
    let data: &[u8] = &vaa.payload;
    let target_chain = data.get_u16(33);
    let chain = data.get_u16(35);

    if (target_chain != CHAIN_ID_NEAR) && (target_chain != 0) {
        refund_and_panic("InvalidREegisterChainChain", refund_to);
    }

    if storage.emitter_registration.contains_key(&chain) {
        refund_and_panic("DuplicateChainRegistration", refund_to);
    }
    let storage_used = env::storage_usage();
    storage
        .emitter_registration
        .insert(&chain, &data[37..69].to_vec());
    let required_cost =
        (Balance::from(env::storage_usage() - storage_used)) * env::storage_byte_cost();

    if required_cost > deposit {
        refund_and_panic("DepositUnderflowForRegistration", refund_to);
    }
    deposit -= required_cost;

    env::log_str(&format!(
        "register chain {} to {}",
        chain,
        hex::encode(&data[37..69])
    ));

    deposit
}

fn vaa_upgrade_contract(
    storage: &mut Portal,
    vaa: &state::ParsedVAA,
    deposit: Balance,
    refund_to: &AccountId,
) -> Balance {
    let data: &[u8] = &vaa.payload;
    let chain = data.get_u16(33);
    if chain != CHAIN_ID_NEAR {
        refund_and_panic("InvalidContractUpgradeChain", refund_to);
    }

    let uh = data.get_bytes32(0);
    env::log_str(&format!(
        "portal/{}#{}: vaa_update_contract: {}",
        file!(),
        line!(),
        hex::encode(&uh)
    ));
    storage.upgrade_hash = uh.to_vec(); // Too lazy to do proper accounting here...
    deposit
}

fn vaa_governance(
    storage: &mut Portal,
    vaa: &state::ParsedVAA,
    gov_idx: u32,
    deposit: Balance,
    refund_to: &AccountId,
) -> Balance {
    if gov_idx != vaa.guardian_set_index {
        refund_and_panic("InvalidGovernanceSet", refund_to);
    }

    if (CHAIN_ID_SOL != vaa.emitter_chain)
        || (hex::decode("0000000000000000000000000000000000000000000000000000000000000004")
            .unwrap()
            != vaa.emitter_address)
    {
        refund_and_panic("InvalidGovernanceEmitter", refund_to);
    }

    let data: &[u8] = &vaa.payload;
    let action = data.get_u8(32);

    match action {
        1u8 => vaa_register_chain(storage, vaa, deposit, refund_to),
        2u8 => vaa_upgrade_contract(storage, vaa, deposit, refund_to),
        _ => refund_and_panic("InvalidGovernanceAction", refund_to),
    }
}

fn vaa_transfer(
    storage: &mut Portal,
    vaa: &state::ParsedVAA,
    action: u8,
    deposit: Balance,
    refund_to: AccountId,
) -> PromiseOrValue<bool> {
    env::log_str(&hex::encode(&vaa.payload));

    let data: &[u8] = &vaa.payload[1..];

    let amount = data.get_u256(0);
    let token_address = data.get_bytes32(32).to_vec();
    let token_chain = data.get_u16(64);
    let recipient = data.get_bytes32(66).to_vec();
    let recipient_chain = data.get_u16(98);
    let fee: (u128, u128) = if action == 1 {
        data.get_u256(100)
    } else {
        (0, 0)
    };

    if recipient_chain != CHAIN_ID_NEAR {
        refund_and_panic("InvalidRecipientChain", &refund_to);
    }

    if !storage.hash_map.contains_key(&recipient) {
        refund_and_panic("UnregisteredReceipient", &refund_to);
    }
    let mr = storage.hash_map.get(&recipient).unwrap();

    env::log_str(&format!(
        "portal/{}#{}: vaa_transfer:  {} {}",
        file!(),
        line!(),
        hex::encode(&token_address),
        token_chain
    ));

    let account = if token_chain == CHAIN_ID_NEAR && token_address == vec![0; 32] {
        env::current_account_id()
    } else {
        let p = token_key(token_address.clone(), token_chain);

        env::log_str(&format!(
            "portal/{}#{}: vaa_transfer:  {}",
            file!(),
            line!(),
            hex::encode(&p)
        ));

        if !storage.key_map.contains_key(&p) {
            refund_and_panic("AssetNotAttested", &refund_to);
        }

        storage.key_map.get(&p).unwrap()
    };

    let mut prom;

    if action == 3 {
        env::log_str(&format!(
            "portal/{}#{}: vaa_transfer:  account:  {}    refund_to: {}   pred: {}   signer: {}  mr: {}",
            file!(),
            line!(),
            account,
            refund_to,
            env::predecessor_account_id(),
            env::signer_account_id(),
            mr
        ));
        refund_and_panic("Payload3NotImplemented", &refund_to);
    }

    if token_chain == CHAIN_ID_NEAR {
        if token_address == vec![0; 32] {
            env::log_str(&format!(
                "portal/{}#{}: vaa_transfer:  deposit {}",
                file!(),
                line!(),
                deposit
            ));
            let namount = amount.1 * NEAR_MULT;
            let nfee = fee.1 * NEAR_MULT;
            if nfee > namount {
                refund_and_panic("nfee > namount", &refund_to);
            }

            // Once you create a Promise, there is no going back..
            if nfee == 0 {
                env::log_str(&format!(
                    "portal/{}#{}: vaa_transfer:  sending {} NEAR to {}",
                    file!(),
                    line!(),
                    namount,
                    mr
                ));
                prom = Promise::new(mr).transfer(namount);
            } else {
                env::log_str(&format!(
                    "portal/{}#{}: vaa_transfer:  sending {} NEAR to {}",
                    file!(),
                    line!(),
                    namount - nfee,
                    mr
                ));
                env::log_str(&format!(
                    "portal/{}#{}: vaa_transfer:  sending {} NEAR to {}",
                    file!(),
                    line!(),
                    nfee,
                    env::signer_account_id()
                ));

                prom = Promise::new(mr)
                    .transfer(namount - nfee)
                    .then(Promise::new(refund_to).transfer(nfee + deposit));
            }
        } else {
            let mut near_mult: u128 = 1;

            let td = if !storage.tokens.contains_key(&account) {
                refund_and_panic("AssetNotAttested2", &refund_to);
            } else {
                storage.tokens.get(&account).unwrap()
            };

            if td.decimals > 8 {
                near_mult = 10_u128.pow(td.decimals as u32 - 8);
            }

            let namount = amount.1 * near_mult;
            let nfee = fee.1 * near_mult;

            env::log_str(&format!(
                "portal/{}#{}: vaa_transfer calling ft_transfer against {} for {} from {} to {}",
                file!(),
                line!(),
                account,
                namount,
                env::current_account_id(),
                mr
            ));

            if namount == 0 {
                refund_and_panic("EmptyTransfer", &refund_to);
            }

            // Once you create a Promise, there is no going back..
            if nfee == 0 {
                prom = ext_ft_contract::ext(account)
                    .with_attached_deposit(1)
                    .ft_transfer(mr, U128::from(namount), None);
            } else {
                prom = ext_ft_contract::ext(account.clone())
                    .with_attached_deposit(1)
                    .ft_transfer(mr, U128::from(namount - nfee), None)
                    .then(
                        ext_ft_contract::ext(account)
                            .with_attached_deposit(1)
                            .ft_transfer(refund_to.clone(), U128::from(nfee), None),
                    );
            }
            if deposit > 0 {
                prom = prom.then(Promise::new(refund_to).transfer(deposit));
            }
        }
    } else {
        // Once you create a Promise, there is no going back..

        prom = ext_ft_contract::ext(account).vaa_transfer(
            amount.1,
            token_address,
            token_chain,
            mr,
            recipient_chain,
            fee.1,
        );

        if deposit > 0 {
            env::log_str(&format!(
                "portal/{}#{}: refund {} to {}",
                file!(),
                line!(),
                deposit,
                refund_to
            ));

            prom = prom.then(Promise::new(refund_to).transfer(deposit));
        }
    }

    PromiseOrValue::Promise(prom)
}

fn refund_and_panic(s: &str, refund_to: &AccountId) -> ! {
    if env::attached_deposit() > 0 {
        env::log_str(&format!(
            "portal/{}#{}: refund {} to {}",
            file!(),
            line!(),
            env::attached_deposit(),
            refund_to
        ));
        Promise::new(refund_to.clone()).transfer(env::attached_deposit());
    }
    env::panic_str(s);
}

fn vaa_asset_meta(
    storage: &mut Portal,
    vaa: &state::ParsedVAA,
    mut deposit: Balance,
    refund_to: AccountId,
) -> PromiseOrValue<bool> {
    env::log_str(&format!(
        "portal/{}#{}: vaa_asset_meta: {} ",
        file!(),
        line!(),
        deposit
    ));

    env::log_str(&hex::encode(&vaa.payload));

    let data: &[u8] = &vaa.payload[1..];

    let token_chain = data.get_u16(32);
    if token_chain == CHAIN_ID_NEAR {
        refund_and_panic("CannotAttestNearAssets", &refund_to);
    }
    let tkey = token_key(data[0..32].to_vec(), token_chain);

    env::log_str(&format!(
        "portal/{}#{}: vaa_asset_meta: {} ",
        file!(),
        line!(),
        hex::encode(&tkey)
    ));

    let fresh;

    let bridge_token_account;

    let mut decimals = data.get_u8(34);

    if storage.key_map.contains_key(&tkey) {
        bridge_token_account = storage.key_map.get(&tkey).unwrap();
        fresh = false;
        env::log_str(&format!("portal/{}#{}: vaa_asset_meta", file!(), line!()));
    } else {
        let storage_used = env::storage_usage();
        storage.last_asset += 1;
        let asset_id = storage.last_asset;
        let account_name = format!("{}.{}", asset_id, env::current_account_id());
        bridge_token_account = AccountId::new_unchecked(account_name.clone());

        let d = TokenData {
            meta: data.to_vec(),
            decimals,
            address: hex::encode(&data[0..32]),
            chain: token_chain,
        };

        env::log_str(&format!(
            "portal/{}#{}: vaa_asset_meta:  {}  ",
            file!(),
            line!(),
            bridge_token_account
        ));

        storage.tokens.insert(&bridge_token_account, &d);
        storage.key_map.insert(&tkey, &bridge_token_account);
        storage
            .hash_map
            .insert(&env::sha256(account_name.as_bytes()), &bridge_token_account);

        fresh = true;

        let required_cost = (Balance::from(env::storage_usage()) - Balance::from(storage_used))
            * env::storage_byte_cost();
        if required_cost > deposit {
            refund_and_panic("DepositUnderflowForToken", &refund_to);
        }

        deposit -= required_cost;
    }

    // Stick some useful meta-data into the asset to allow us to map backwards from a on-chain asset to the wormhole meta data
    let reference = hex::encode(&tkey);
    let ref_hash = env::sha256(reference.as_bytes());

    let symbol = data.get_bytes32(35).to_vec();
    let name = data.get_bytes32(67).to_vec();
    let wname = get_string_from_32(&name) + " (Wormhole)";

    // Decimals are capped at 8 in wormhole
    if decimals > 8 {
        decimals = 8;
    }

    let ft = FungibleTokenMetadata {
        spec: FT_METADATA_SPEC.to_string(),
        name: wname,
        symbol: get_string_from_32(&symbol),
        icon: Some("".to_string()), // Is there ANY way to supply this?
        reference: Some(reference),
        reference_hash: Some(Base64VecU8::from(ref_hash)),
        decimals,
    };

    let mut p = if !fresh {
        env::log_str(&format!("portal/{}#{}: vaa_asset_meta", file!(), line!()));
        ext_ft_contract::ext(bridge_token_account.clone()).update_ft(
            ft,
            data.to_vec(),
            vaa.sequence,
        )
    } else {
        env::log_str(&format!("portal/{}#{}: vaa_asset_meta", file!(), line!()));
        let cost = (TRANSFER_BUFFER + BRIDGE_TOKEN_BINARY.len() as u128) * env::storage_byte_cost();

        if cost > deposit {
            refund_and_panic("PrecheckFailedDepositUnderFlow", &refund_to);
        }

        deposit -= cost;

        Promise::new(bridge_token_account.clone())
            .create_account()
            .transfer(cost)
            .add_full_access_key(storage.owner_pk.clone())
            .deploy_contract(BRIDGE_TOKEN_BINARY.to_vec())
            // Lets initialize it with useful stuff
            .then(ext_ft_contract::ext(bridge_token_account.clone()).new(
                ft,
                data.to_vec(),
                vaa.sequence,
            ))
        // And then lets tell us we are done!
    };

    if deposit > 0 {
        env::log_str(&format!(
            "portal/{}#{}: refund {} to {}",
            file!(),
            line!(),
            deposit,
            env::predecessor_account_id()
        ));
        p = p.then(Promise::new(refund_to).transfer(deposit));
    }

    PromiseOrValue::Promise(p.then(
        ext_portal::ext(env::current_account_id()).finish_deploy(bridge_token_account.clone()),
    ))
}

fn token_key(address: Vec<u8>, chain: u16) -> Vec<u8> {
    [address, chain.to_be_bytes().to_vec()].concat()
}

#[near_bindgen]
impl Portal {
    pub fn emitter(&self) -> (String, String) {
        let acct = env::current_account_id();
        let astr = acct.to_string();

        (astr.clone(), hex::encode(env::sha256(astr.as_bytes())))
    }

    pub fn is_wormhole(&self, token: &String) -> bool {
        let astr = format!(".{}", env::current_account_id().as_str());
        token.ends_with(&astr)
    }

    pub fn deposit_estimates(&self) -> (String, String) {
        let cost =
            ((TRANSFER_BUFFER * 2) + BRIDGE_TOKEN_BINARY.len() as u128) * env::storage_byte_cost();

        (env::storage_byte_cost().to_string(), cost.to_string())
    }

    pub fn get_original_asset(&self, token: String) -> (String, u16) {
        let account = AccountId::new_unchecked(token);

        if !self.tokens.contains_key(&account) {
            env::panic_str("UnknownAssetId");
        }

        let t = self.tokens.get(&account).unwrap();
        (t.address, t.chain)
    }

    pub fn get_foreign_asset(&self, address: String, chain: u16) -> String {
        let p = token_key(hex::decode(address).unwrap(), chain);

        if self.key_map.contains_key(&p) {
            return self.key_map.get(&p).unwrap().to_string();
        }

        "".to_string()
    }

    #[payable]
    pub fn register_account(&mut self, account: String) -> String {
        let storage_used = env::storage_usage();
        let refund_to = env::predecessor_account_id();

        let account_hash = env::sha256(account.as_bytes());
        let ret = hex::encode(&account_hash);

        if self.hash_map.contains_key(&account_hash) {
            Promise::new(refund_to).transfer(env::attached_deposit());
            return ret;
        }
        let a = AccountId::new_unchecked(account);
        self.hash_map.insert(&account_hash, &a);

        if env::storage_usage() < storage_used {
            refund_and_panic("ImpossibleStorage", &refund_to);
        }

        let required_cost =
            (Balance::from(env::storage_usage() - storage_used)) * env::storage_byte_cost();
        let mut deposit = env::attached_deposit();
        if required_cost > deposit {
            refund_and_panic("DepositUnderflowForToken2", &refund_to);
        }

        deposit -= required_cost;

        if deposit > 0 {
            env::log_str(&format!(
                "portal/{}#{}: refund {} to {}",
                file!(),
                line!(),
                deposit,
                env::predecessor_account_id()
            ));

            Promise::new(env::predecessor_account_id()).transfer(deposit);
        }

        ret
    }

    #[payable]
    pub fn register_bank(&mut self) -> PromiseOrValue<bool> {
        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        let refund_to = env::predecessor_account_id();
        let mut deposit = env::attached_deposit();

        if !self.bank.contains_key(&refund_to) {
            let b = 0;

            let storage_used = env::storage_usage();

            self.bank.insert(&refund_to, &b);

            let required_cost =
                (Balance::from(env::storage_usage() - storage_used)) * env::storage_byte_cost();

            if required_cost > deposit {
                refund_and_panic("DepositUnderflowForRegistration", &refund_to);
            }
            deposit -= required_cost;
        }

        if deposit > 0 {
            PromiseOrValue::Promise(Promise::new(refund_to).transfer(deposit))
        } else {
            PromiseOrValue::Value(false)
        }
    }

    #[payable]
    pub fn fill_bank(&mut self) {
        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        let refund_to = env::predecessor_account_id();

        if !self.bank.contains_key(&refund_to) {
            refund_and_panic("UnregisteredAccount", &refund_to);
        }

        let b = self.bank.get(&refund_to).unwrap() + env::attached_deposit();
        self.bank.insert(&refund_to, &b);
    }

    #[payable]
    pub fn drain_bank(&mut self) -> Promise {
        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        let refund_to = env::predecessor_account_id();
        if env::attached_deposit() != 1 {
            refund_and_panic("unauthorized", &refund_to);
        }

        if !self.bank.contains_key(&refund_to) {
            refund_and_panic("UnregisteredAccount", &refund_to);
        }

        let b = self.bank.get(&refund_to).unwrap();
        let nv = 0;
        self.bank.insert(&refund_to, &nv);

        Promise::new(refund_to).transfer(b)
    }

    pub fn bank_balance(&self) -> (bool, Balance) {
        let acct = env::predecessor_account_id();
        if self.bank.contains_key(&acct) {
            (true, self.bank.get(&acct).unwrap())
        } else {
            (false, 0)
        }
    }

    pub fn hash_account(&self, account: String) -> (bool, String) {
        // Yes, you could hash it yourself but then you wouldn't know
        // if it was already registered...
        let account_hash = env::sha256(account.as_bytes());
        let ret = hex::encode(&account_hash);
        (self.hash_map.contains_key(&account_hash), ret)
    }

    pub fn hash_lookup(&self, hash: String) -> (bool, String) {
        let account_hash = hex::decode(&hash).unwrap();
        if self.hash_map.contains_key(&account_hash) {
            (true, self.hash_map.get(&account_hash).unwrap().to_string())
        } else {
            (false, "".to_string())
        }
    }

    #[payable]
    pub fn send_transfer_near(
        &mut self,
        receiver: String,
        chain: u16,
        fee: String,
        payload: String,
        message_fee: Balance,
    ) -> Promise {
        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        if message_fee > env::attached_deposit() {
            refund_and_panic("MessageFeeExceedsDeposit", &env::predecessor_account_id());
        }

        let amount = env::attached_deposit() - message_fee;

        let namount = amount / NEAR_MULT;
        let nfee = fee.parse::<u128>().unwrap() / NEAR_MULT;

        if namount == 0 {
            refund_and_panic("EmptyTransfer", &env::predecessor_account_id());
        }
        //let dust = amount - (namount * NEAR_MULT) - (nfee * NEAR_MULT);

        let mut p = [
            // PayloadID uint8 = 1
            (if payload.is_empty() { 1 } else { 3 } as u8)
                .to_be_bytes()
                .to_vec(),
            // Amount uint256
            vec![0; 24],
            (namount as u64).to_be_bytes().to_vec(),
            //TokenAddress bytes32
            vec![0; 32],
            // TokenChain uint16
            (CHAIN_ID_NEAR as u16).to_be_bytes().to_vec(),
            // To bytes32
            vec![0; (64 - receiver.len()) / 2],
            hex::decode(receiver).unwrap(),
            // ToChain uint16
            (chain as u16).to_be_bytes().to_vec(),
        ]
        .concat();

        if payload.is_empty() {
            p = [p, vec![0; 24], (nfee as u64).to_be_bytes().to_vec()].concat();
            if p.len() != 133 {
                refund_and_panic("Payload1 formatting error", &env::predecessor_account_id());
            }
        } else {
            p = [p, hex::decode(&payload).unwrap()].concat();
            if p.len() != (133 + (payload.len() / 2)) {
                refund_and_panic("Payload3 formatting error", &env::predecessor_account_id());
            }
        }

        ext_worm_hole::ext(self.core.clone())
            .with_attached_deposit(message_fee)
            .publish_message(hex::encode(p), env::block_height() as u32)
    }

    #[payable]
    pub fn send_transfer_wormhole_token(
        &mut self,
        amount: String,
        token: String,
        receiver: String,
        chain: u16,
        fee: String,
        payload: String,
        message_fee: Balance,
    ) -> Promise {
        if (message_fee > 0) && (env::attached_deposit() < message_fee)
            || (env::attached_deposit() == 0)
        {
            refund_and_panic("DepositRequired", &env::predecessor_account_id());
        }

        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        if self.is_wormhole(&token) {
            ext_ft_contract::ext(AccountId::try_from(token).unwrap())
                .vaa_withdraw(
                    env::predecessor_account_id(),
                    amount.parse().unwrap(),
                    receiver,
                    chain,
                    fee.parse().unwrap(),
                    payload,
                )
                .then(
                    Self::ext(env::current_account_id())
                        .with_attached_deposit(env::attached_deposit())
                        .send_transfer_token_wormhole_callback(message_fee),
                )
        } else {
            env::panic_str("NotWormhole");
        }
    }

    #[private]
    #[payable]
    pub fn send_transfer_token_wormhole_callback(
        &mut self,
        message_fee: Balance,
        #[callback_result] payload: Result<String, PromiseError>,
    ) -> Promise {
        if payload.is_err() {
            env::panic_str("PayloadError");
        }

        if env::attached_deposit() < message_fee {
            env::panic_str("DepositUnderflow");
        }

        ext_worm_hole::ext(self.core.clone())
            .with_attached_deposit(message_fee)
            .publish_message(payload.unwrap(), env::block_height() as u32)
    }

    pub fn is_transfer_completed(&self, vaa: String) -> bool {
        let h = hex::decode(vaa).expect("invalidVaa");
        let pvaa = state::ParsedVAA::parse(&h);

        self.dups.contains(&pvaa.hash)
    }

    #[payable]
    pub fn submit_vaa(&mut self, vaa: String) -> Promise {
        let refund_to = env::predecessor_account_id();

        if env::prepaid_gas() < Gas(300_000_000_000_000) {
            refund_and_panic("NotEnoughGas", &refund_to);
        }

        if env::attached_deposit() < (TRANSFER_BUFFER * env::storage_byte_cost()) {
            refund_and_panic("StorageDepositUnderflow", &refund_to);
        }

        ext_worm_hole::ext(self.core.clone())
            .verify_vaa(vaa.clone())
            .then(
                Self::ext(env::current_account_id())
                    .with_unused_gas_weight(10)
                    .with_attached_deposit(env::attached_deposit())
                    .submit_vaa_callback(vaa, refund_to),
            )
    }

    #[private] // So, all of wormhole security rests in this one statement?
    #[payable]
    pub fn submit_vaa_callback(
        &mut self,
        vaa: String,
        refund_to: AccountId,
        #[callback_result] gov_idx: Result<u32, PromiseError>,
    ) -> PromiseOrValue<bool> {
        env::log_str(&format!(
            "portal/{}#{}: submit_vaa_callback: {}  {} used: {}  prepaid: {}",
            file!(),
            line!(),
            env::attached_deposit(),
            env::predecessor_account_id(),
            serde_json::to_string(&env::used_gas()).unwrap(),
            serde_json::to_string(&env::prepaid_gas()).unwrap()
        ));

        if gov_idx.is_err() {
            refund_and_panic("vaaVerifyFail", &refund_to);
        }

        let vaa_str = hex::decode(&vaa);
        let h = match vaa_str {
            Ok(v) => v,
            Err(e) => refund_and_panic(&e.to_string(), &refund_to),
        };

        let pvaa = state::ParsedVAA::parse(&h);

        if pvaa.version != 1 {
            refund_and_panic("invalidVersion", &refund_to);
        }

        let data: &[u8] = &pvaa.payload;

        let governance = data[0..32]
            == hex::decode("000000000000000000000000000000000000000000546f6b656e427269646765")
                .unwrap();
        let action = data.get_u8(0);

        let storage_used = env::storage_usage();
        let mut deposit = env::attached_deposit();

        // Check if VAA with this hash was already accepted
        if self.dups.contains(&pvaa.hash) {
            refund_and_panic("alreadyExecuted", &refund_to);
        }
        self.dups.insert(&pvaa.hash);

        let required_cost =
            (Balance::from(env::storage_usage() - storage_used)) * env::storage_byte_cost();
        if required_cost > deposit {
            refund_and_panic("DepositUnderflowForHash", &refund_to);
        }
        deposit -= required_cost;

        if governance {
            let bal = vaa_governance(self, &pvaa, gov_idx.unwrap(), deposit, &refund_to);
            if bal > 0 {
                env::log_str(&format!(
                    "portal/{}#{}: refunding {} to {}",
                    file!(),
                    line!(),
                    bal,
                    refund_to
                ));

                return PromiseOrValue::Promise(Promise::new(refund_to).transfer(bal));
            }
            return PromiseOrValue::Value(true);
        }

        env::log_str(&format!("looking up chain {}", pvaa.emitter_chain));

        if !self.emitter_registration.contains_key(&pvaa.emitter_chain) {
            refund_and_panic("ChainNotRegistered", &refund_to);
        }

        if self.emitter_registration.get(&pvaa.emitter_chain).unwrap() != pvaa.emitter_address {
            refund_and_panic("InvalidRegistration", &refund_to);
        }

        match action {
            1u8 => vaa_transfer(self, &pvaa, action, deposit, refund_to.clone()),
            2u8 => vaa_asset_meta(self, &pvaa, deposit, refund_to.clone()),
            3u8 => vaa_transfer(self, &pvaa, action, deposit, refund_to.clone()),
            _ => {
                refund_and_panic("invalidPortAction", &refund_to);
            }
        }
    }

    #[payable]
    pub fn attest_near(&mut self, message_fee: Balance) -> Promise {
        if (message_fee > 0) && (env::attached_deposit() < message_fee)
            || (env::attached_deposit() == 0)
        {
            refund_and_panic("DepositRequired", &env::predecessor_account_id());
        }

        require!(
            env::prepaid_gas() >= Gas(100_000_000_000_000),
            &format!(
                "portal/{}#{}: more gas is required {}",
                file!(),
                line!(),
                serde_json::to_string(&env::prepaid_gas()).unwrap()
            )
        );

        let p = [
            (2_u8).to_be_bytes().to_vec(),
            vec![0; 32],
            (CHAIN_ID_NEAR as u16).to_be_bytes().to_vec(),
            (24_u8).to_be_bytes().to_vec(), // yectoNEAR is 1e24 ...
            byte_utils::extend_string_to_32("NEAR"),
            byte_utils::extend_string_to_32("NEAR"),
        ]
        .concat();

        if p.len() != 100 {
            env::log_str(&format!("len: {}  val: {}", p.len(), hex::encode(p)));
            env::panic_str("Formatting error");
        }

        ext_worm_hole::ext(self.core.clone())
            .with_attached_deposit(env::attached_deposit())
            .publish_message(hex::encode(p), env::block_height() as u32)
    }

    #[payable]
    pub fn attest_token(&mut self, token: String, message_fee: Balance) -> Promise {
        if (message_fee > 0) && (env::attached_deposit() < message_fee)
            || (env::attached_deposit() == 0)
        {
            refund_and_panic("DepositRequired", &env::predecessor_account_id());
        }

        if env::prepaid_gas() < Gas(100_000_000_000_000) {
            refund_and_panic("MoreGasRequired", &env::predecessor_account_id());
        }

        if self.is_wormhole(&token) {
            refund_and_panic("CannotAttestAWormholeToken", &env::predecessor_account_id())
        } else {
            env::log_str(&format!("portal/{}#{}", file!(), line!()));

            ext_ft_contract::ext(AccountId::try_from(token.clone()).unwrap())
                .ft_metadata()
                .then(
                    Self::ext(env::current_account_id())
                        .with_unused_gas_weight(10)
                        .with_attached_deposit(env::attached_deposit())
                        .attest_token_callback(token, env::predecessor_account_id(), message_fee),
                )
        }
    }

    #[payable]
    #[private]
    pub fn attest_token_callback(
        &mut self,
        token: String,
        refund_to: AccountId,
        message_fee: Balance,
        #[callback_result] ft_info: Result<FungibleTokenMetadata, PromiseError>,
    ) -> Promise {
        if ft_info.is_err() {
            refund_and_panic("FailedToRetrieveMetaData", &refund_to);
        }

        let ft = ft_info.unwrap();

        let bridge_token_account = AccountId::new_unchecked(token.clone());
        let account_hash = env::sha256(token.as_bytes());
        let tkey = token_key(account_hash.to_vec(), CHAIN_ID_NEAR);

        let mut deposit = env::attached_deposit();

        let storage_used = env::storage_usage();

        if !self.tokens.contains_key(&bridge_token_account) {
            let d = TokenData {
                meta: b"".to_vec(),
                decimals: ft.decimals,
                address: hex::encode(&account_hash),
                chain: CHAIN_ID_NEAR,
            };
            self.tokens.insert(&bridge_token_account, &d);
        }

        self.key_map.insert(&tkey, &bridge_token_account);
        self.hash_map.insert(&account_hash, &bridge_token_account);

        let required_cost =
            (Balance::from(env::storage_usage() - storage_used)) * env::storage_byte_cost();

        if required_cost > deposit {
            env::log_str(&format!(
                "portal/{}#{}: attest_token_callback: {} {}",
                file!(),
                line!(),
                required_cost,
                env::attached_deposit()
            ));

            refund_and_panic("DepositUnderflowForRegistration", &refund_to);
        }
        deposit -= required_cost;

        let p = [
            (2_u8).to_be_bytes().to_vec(),
            account_hash,
            (CHAIN_ID_NEAR as u16).to_be_bytes().to_vec(),
            (ft.decimals as u8).to_be_bytes().to_vec(), // yectoNEAR is 1e24 ...
            byte_utils::extend_string_to_32(&ft.symbol),
            byte_utils::extend_string_to_32(&ft.name),
        ]
        .concat();

        if p.len() != 100 {
            env::log_str(&format!("len: {}  val: {}", p.len(), hex::encode(p)));
            refund_and_panic("formatting error", &refund_to);
        }

        if deposit < message_fee {
            refund_and_panic("MessageFeeUnderflow", &refund_to);
        }

        deposit -= message_fee;

        let mut prom = ext_worm_hole::ext(self.core.clone())
            .with_attached_deposit(message_fee)
            .publish_message(hex::encode(p), env::block_height() as u32);

        if deposit > 0 {
            env::log_str(&format!(
                "portal/{}#{}: refunding {} to {}",
                file!(),
                line!(),
                deposit,
                refund_to
            ));

            prom = prom.then(Promise::new(refund_to).transfer(deposit));
        }

        prom
    }

    #[private]
    pub fn finish_deploy(&mut self, token: AccountId) -> String {
        if is_promise_success() {
            token.to_string()
        } else {
            env::panic_str("bad deploy");
        }
    }

    #[private]
    pub fn ft_on_transfer_callback(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
        token: AccountId,
        #[callback_result] ft_info: Result<FungibleTokenMetadata, PromiseError>,
    ) -> PromiseOrValue<U128> {
        env::log_str(&format!(
            "portal/{}#{}: ft_on_transfer_callback: {} {} {}",
            file!(),
            line!(),
            sender_id,
            msg,
            token
        ));

        if env::signer_account_id() != sender_id {
            env::panic_str("signer != sender");
        }

        if ft_info.is_err() {
            env::panic_str("ft_infoError");
        }

        let ft = ft_info.unwrap();
        let tp: TransferMsgPayload = near_sdk::serde_json::from_str(&msg).unwrap();

        if tp.message_fee > 0 {
            if !self.bank.contains_key(&sender_id) {
                env::panic_str("senderHasNoBank");
            }
        }

        let mut near_mult: u128 = 1;

        if ft.decimals > 8 {
            near_mult = 10_u128.pow(ft.decimals as u32 - 8);
        }

        let namount = u128::from(amount) / near_mult;
        let nfee = tp.fee.parse::<u128>().unwrap() / near_mult;

        if namount == 0 {
            env::panic_str("EmptyTransfer");
        }

        let mut p = [
            // PayloadID uint8 = 1
            (if tp.payload.is_empty() { 1 } else { 3 } as u8)
                .to_be_bytes()
                .to_vec(),
            // Amount uint256
            vec![0; 24],
            (namount as u64).to_be_bytes().to_vec(),
            //TokenAddress bytes32
            env::sha256(token.to_string().as_bytes()),
            // TokenChain uint16
            (CHAIN_ID_NEAR as u16).to_be_bytes().to_vec(),
            // To bytes32
            vec![0; (64 - tp.receiver.len()) / 2],
            hex::decode(tp.receiver).unwrap(),
            // ToChain uint16
            (tp.chain as u16).to_be_bytes().to_vec(),
        ]
        .concat();

        if tp.payload.is_empty() {
            p = [p, vec![0; 24], (nfee as u64).to_be_bytes().to_vec()].concat();
            if p.len() != 133 {
                env::panic_str(&format!("paylod1 formatting errro  len = {}", p.len()));
            }
        } else {
            p = [p, hex::decode(&tp.payload).unwrap()].concat();
            if p.len() != (133 + (tp.payload.len() / 2)) {
                env::panic_str(&format!("paylod3 formatting errro  len = {}", p.len()));
            }
        }

        if tp.message_fee > 0 {
            let mut b = self.bank.get(&sender_id).unwrap();
            if b < tp.message_fee {
                env::panic_str("bank underflow");
            }
            b -= tp.message_fee;
            self.bank.insert(&sender_id, &b);
        }

        PromiseOrValue::Promise(
            ext_worm_hole::ext(self.core.clone())
                .with_attached_deposit(tp.message_fee)
                .publish_message(hex::encode(p), env::block_height() as u32)
                .then(Self::ext(env::current_account_id()).emitter_callback_pov()),
        )
    }

    #[private]
    pub fn emitter_callback_pov(
        &mut self,
        #[callback_result] seq: Result<u64, PromiseError>,
    ) -> PromiseOrValue<U128> {
        env::log_str(&format!(
            "portal/{}#{}: emitter_callback_pov",
            file!(),
            line!()
        ));

        if seq.is_err() {
            env::panic_str("EmitFail");
        }

        PromiseOrValue::Value(U128::from(0))
    }

    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        env::log_str(&format!(
            "portal/{}#{}: ft_on_transfer attached_deposit:  {}",
            file!(),
            line!(),
            env::attached_deposit()
        ));

        // require!(env::prepaid_gas() >= GAS_FOR_FT_TRANSFER_CALL, "More gas is required");

        PromiseOrValue::Promise(
            ext_ft_contract::ext(env::predecessor_account_id())
                .ft_metadata()
                .then(
                    Self::ext(env::current_account_id()).ft_on_transfer_callback(
                        sender_id,
                        amount,
                        msg,
                        env::predecessor_account_id(),
                    ),
                ),
        )
    }

    pub fn boot_portal(&mut self, core: String) {
        if self.owner_pk != env::signer_account_pk() {
            env::panic_str("invalidSigner");
        }

        if self.booted {
            env::panic_str("NoDonut");
        }
        self.booted = true;
        self.core = AccountId::try_from(core).unwrap();

        let account_hash = env::sha256(env::current_account_id().to_string().as_bytes());
        env::log_str(&format!("portal emitter: {}", hex::encode(account_hash)));
    }

    #[private]
    pub fn update_contract_done(
        &mut self,
        refund_to: near_sdk::AccountId,
        storage_used: u64,
        attached_deposit: u128,
    ) {
        let delta = (env::storage_usage() as i128 - storage_used as i128)
            * env::storage_byte_cost() as i128;
        let refund = attached_deposit as i128 - delta;
        if refund > 0 {
            env::log_str(&format!(
                "portal/{}#{}: update_contract_done: refund {} to {}",
                file!(),
                line!(),
                refund,
                refund_to
            ));
            Promise::new(refund_to).transfer(refund as u128);
        }
    }

    #[private]
    fn update_contract_work(&mut self, v: Vec<u8>) -> Promise {
        if env::attached_deposit() == 0 {
            env::panic_str("attach some cash");
        }

        let s = env::sha256(&v);

        env::log_str(&format!(
            "portal/{}#{}: update_contract: {}",
            file!(),
            line!(),
            hex::encode(&s)
        ));

        if s.to_vec() != self.upgrade_hash {
            if env::attached_deposit() > 0 {
                env::log_str(&format!(
                    "portal/{}#{}: refunding {} to {}",
                    file!(),
                    line!(),
                    env::attached_deposit(),
                    env::predecessor_account_id()
                ));

                Promise::new(env::predecessor_account_id()).transfer(env::attached_deposit());
            }
            env::panic_str("invalidUpgradeContract");
        }

        Promise::new(env::current_account_id())
            .deploy_contract(v.to_vec())
            .then(Self::ext(env::current_account_id()).update_contract_done(
                env::predecessor_account_id(),
                env::storage_usage(),
                env::attached_deposit(),
            ))
    }
}

//  let result = await userAccount.functionCall({
//    contractId: config.tokenAccount,
//    methodName: "update_contract",
//    args: wormholeContract,
//    attachedDeposit: "12500000000000000000000",
//    gas: 300000000000000,
//  });

#[no_mangle]
pub extern "C" fn update_contract() {
    env::setup_panic_hook();
    let mut contract: Portal = env::state_read().expect("Contract is not initialized");
    contract.update_contract_work(env::input().unwrap());
}