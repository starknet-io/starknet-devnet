//! Encoding of classes declared after genesis and reconstruction of states from state diffs.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::state::cached_state::CachedState;
use cairo_lang_starknet_classes::contract_class::ContractClass as SierraContractClass;
use serde::{Deserialize, Serialize};
use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
use starknet_rs_core::types::Felt;
use starknet_types::contract_address::ContractAddress;
use starknet_types::contract_class::deprecated::json_contract_class::Cairo0Json;
use starknet_types::contract_class::deprecated::rpc_contract_class::DeprecatedContractClass;
use starknet_types::contract_class::{Cairo0ContractClass, ContractClass};
use starknet_types::patricia_key::StorageKey;
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::blocks::StarknetBlocks;
use crate::error::{DevnetResult, Error};
use crate::state::state_diff::StateDiff;
use crate::state::state_readers::DictState;
use crate::state::{CommittedClassStorage, StarknetState};

/// A class definition, preserving the representation Devnet stores it in.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "definition", rename_all = "snake_case")]
pub(super) enum ClassDump {
    Cairo0Json(serde_json::Value),
    Cairo0Rpc(DeprecatedContractClass),
    Cairo1(Box<SierraContractClass>),
}

impl From<&ContractClass> for ClassDump {
    fn from(class: &ContractClass) -> Self {
        match class {
            ContractClass::Cairo0(Cairo0ContractClass::RawJson(json)) => {
                Self::Cairo0Json(json.inner.clone())
            }
            ContractClass::Cairo0(Cairo0ContractClass::Rpc(rpc)) => Self::Cairo0Rpc(rpc.clone()),
            ContractClass::Cairo1(sierra) => Self::Cairo1(Box::new(sierra.clone())),
        }
    }
}

impl From<ClassDump> for ContractClass {
    fn from(class: ClassDump) -> Self {
        match class {
            ClassDump::Cairo0Json(inner) => {
                ContractClass::Cairo0(Cairo0ContractClass::RawJson(Cairo0Json { inner }))
            }
            ClassDump::Cairo0Rpc(rpc) => ContractClass::Cairo0(Cairo0ContractClass::Rpc(rpc)),
            ClassDump::Cairo1(sierra) => ContractClass::Cairo1(*sierra),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassDefinitionDump {
    class_hash: Felt,
    class: ClassDump,
}

/// Collects the class definitions a dump stores, so each is stored once.
#[derive(Default)]
pub(super) struct ClassTable(BTreeMap<Felt, ClassDump>);

impl ClassTable {
    pub(super) fn insert(&mut self, class_hash: Felt, class: &ContractClass) {
        self.0.entry(class_hash).or_insert_with(|| ClassDump::from(class));
    }
}

/// Classes that the genesis of the configuration does not produce.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClassesDump {
    /// Definitions of classes declared after genesis and of configured classes that are not
    /// built into Devnet, ordered by class hash.
    definitions: Vec<ClassDefinitionDump>,
    /// Classes committed after genesis, with the number of the block they were committed at.
    committed: Vec<(Felt, u64)>,
    /// Classes declared in the pre-confirmed block, not yet committed.
    staging: Vec<Felt>,
}

impl ClassesDump {
    /// Encodes classes committed after `genesis_block_number`; definitions are added to `table`
    /// and filled in by `finish`.
    pub(super) fn encode(
        storage: &CommittedClassStorage,
        genesis_block_number: u64,
        table: &mut ClassTable,
    ) -> Self {
        let CommittedClassStorage {
            staging,
            committed,
            // Derived from `committed`; the genesis entry comes from the genesis.
            block_number_to_classes: _,
        } = storage;

        let mut committed: Vec<_> = committed
            .iter()
            .filter(|(_, (_, block))| *block > genesis_block_number)
            .map(|(class_hash, (class, block))| {
                table.insert(*class_hash, class);
                (*class_hash, *block)
            })
            .collect();
        committed.sort_unstable();
        let staging = sorted(staging.iter().map(|(class_hash, class)| {
            table.insert(*class_hash, class);
            *class_hash
        }));

        Self { definitions: vec![], committed, staging }
    }

    pub(super) fn finish(mut self, table: ClassTable) -> Self {
        self.definitions = table
            .0
            .into_iter()
            .map(|(class_hash, class)| ClassDefinitionDump { class_hash, class })
            .collect();
        self
    }

    /// Returns the stored class definitions and the classes declared after genesis.
    pub(super) fn decode(self) -> DevnetResult<(ClassDefinitions, PostGenesisClasses)> {
        let Self { definitions, committed, staging } = self;

        let mut by_hash = HashMap::with_capacity(definitions.len());
        for ClassDefinitionDump { class_hash, class } in definitions {
            if by_hash.insert(class_hash, ContractClass::from(class)).is_some() {
                return Err(invalid_dump(format!(
                    "class {class_hash:#x} is defined more than once"
                )));
            }
        }
        Ok((ClassDefinitions(by_hash), PostGenesisClasses { committed, staging }))
    }
}

/// Classes declared after genesis, referring to stored definitions.
pub(super) struct PostGenesisClasses {
    committed: Vec<(Felt, u64)>,
    staging: Vec<Felt>,
}

impl PostGenesisClasses {
    /// Adds the classes to the class storage produced by the genesis.
    pub(super) fn add_to(
        self,
        storage: &mut CommittedClassStorage,
        definitions: &ClassDefinitions,
    ) -> DevnetResult<()> {
        let Self { committed, staging } = self;
        for (class_hash, block) in committed {
            let class = definitions.get(class_hash)?.clone();
            if storage.committed.insert(class_hash, (class, block)).is_some() {
                return Err(invalid_dump(format!(
                    "class {class_hash:#x} is already declared at genesis"
                )));
            }
            storage.block_number_to_classes.entry(block).or_default().push(class_hash);
        }
        for class_hash in staging {
            storage.staging.insert(class_hash, definitions.get(class_hash)?.clone());
        }
        Ok(())
    }
}

/// Stored class definitions, keyed by class hash.
pub(super) struct ClassDefinitions(HashMap<Felt, ContractClass>);

impl ClassDefinitions {
    pub(super) fn get(&self, class_hash: Felt) -> DevnetResult<&ContractClass> {
        self.0
            .get(&class_hash)
            .ok_or_else(|| invalid_dump(format!("missing definition of class {class_hash:#x}")))
    }
}

/// Returns runnable classes, compiling each stored definition at most once and reusing the classes
/// the genesis already compiled.
pub(super) struct CompiledClasses<'a> {
    definitions: &'a ClassDefinitions,
    compiled: HashMap<Felt, RunnableCompiledClass>,
}

impl<'a> CompiledClasses<'a> {
    pub(super) fn new(definitions: &'a ClassDefinitions, genesis: &StarknetState) -> Self {
        let compiled = genesis
            .historic_state
            .class_hash_to_class
            .iter()
            .map(|(class_hash, class)| (class_hash.0, class.clone()))
            .collect();
        Self { definitions, compiled }
    }

    fn get(&mut self, class_hash: Felt) -> DevnetResult<RunnableCompiledClass> {
        if let Some(compiled) = self.compiled.get(&class_hash) {
            return Ok(compiled.clone());
        }
        let compiled: RunnableCompiledClass =
            self.definitions.get(class_hash)?.clone().try_into()?;
        self.compiled.insert(class_hash, compiled.clone());
        Ok(compiled)
    }
}

/// Entries of the pre-confirmed state diff that the latest state already contains. Creating a block
/// sets the latest state after the next block's pre-processing (which stores an old block hash),
/// so those writes are in both; after a block abort they are in neither.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LatestDump {
    /// Storage slots with the block of their latest update in the latest state.
    storage: Vec<(Felt, Felt, Option<u64>)>,
    nonces: Vec<Felt>,
    class_hashes: Vec<Felt>,
    compiled_class_hashes: Vec<Felt>,
    classes: Vec<Felt>,
}

impl LatestDump {
    pub(super) fn encode(latest: &StarknetState, pre_confirmed_state_diff: &StateDiff) -> Self {
        let layer = Layer::of(latest);
        let StateDiff {
            storage_updates,
            address_to_nonce,
            address_to_class_hash,
            class_hash_to_compiled_class_hash,
            declared_contracts,
            cairo_0_declared_contracts,
            replaced_classes: _,
        } = pre_confirmed_state_diff;

        let contained =
            |map: &BTreeMap<Felt, Felt>, key: Felt, value: Felt| map.get(&key) == Some(&value);
        Self {
            storage: sorted(storage_updates.iter().flat_map(|(address, updates)| {
                let layer = &layer;
                updates.iter().filter_map(move |(key, value)| {
                    let slot = (Felt::from(*address), key.to_felt());
                    (layer.storage.get(&slot) == Some(value))
                        .then(|| (slot.0, slot.1, layer.storage_metadata.get(&slot).copied()))
                })
            })),
            nonces: sorted(
                address_to_nonce
                    .iter()
                    .map(|(address, nonce)| (Felt::from(*address), *nonce))
                    .filter(|(address, nonce)| contained(&layer.nonces, *address, *nonce))
                    .map(|(address, _)| address),
            ),
            class_hashes: sorted(
                address_to_class_hash
                    .iter()
                    .map(|(address, class_hash)| (Felt::from(*address), *class_hash))
                    .filter(|(address, class_hash)| {
                        contained(&layer.class_hashes, *address, *class_hash)
                    })
                    .map(|(address, _)| address),
            ),
            compiled_class_hashes: sorted(
                class_hash_to_compiled_class_hash
                    .iter()
                    .filter(|(class_hash, compiled)| {
                        contained(&layer.compiled_class_hashes, **class_hash, **compiled)
                    })
                    .map(|(class_hash, _)| *class_hash),
            ),
            classes: sorted(
                declared_contracts
                    .iter()
                    .chain(cairo_0_declared_contracts)
                    .filter(|class_hash| layer.classes.contains(class_hash))
                    .copied(),
            ),
        }
    }

    /// Applies the listed entries of `pre_confirmed_state_diff` to `layer`.
    fn apply(self, layer: &mut Layer, pre_confirmed_state_diff: &StateDiff) -> DevnetResult<()> {
        let Self { storage, nonces, class_hashes, compiled_class_hashes, classes } = self;
        let missing = |what: &str, key: Felt| {
            invalid_dump(format!("latest {what} {key:#x} is not in the pre-confirmed state diff"))
        };

        for (address, key, block) in storage {
            let value = pre_confirmed_state_diff
                .storage_updates
                .get(&ContractAddress::new(address)?)
                .and_then(|updates| updates.get(&StorageKey::new(key).ok()?))
                .ok_or_else(|| missing("storage slot of", address))?;
            layer.storage.insert((address, key), *value);
            if let Some(block) = block {
                layer.storage_metadata.insert((address, key), block);
            }
        }
        for address in nonces {
            let nonce = pre_confirmed_state_diff
                .address_to_nonce
                .get(&ContractAddress::new(address)?)
                .ok_or_else(|| missing("nonce of", address))?;
            layer.nonces.insert(address, *nonce);
        }
        for address in class_hashes {
            let class_hash = pre_confirmed_state_diff
                .address_to_class_hash
                .get(&ContractAddress::new(address)?)
                .ok_or_else(|| missing("class hash of", address))?;
            layer.class_hashes.insert(address, *class_hash);
        }
        for class_hash in compiled_class_hashes {
            let compiled = pre_confirmed_state_diff
                .class_hash_to_compiled_class_hash
                .get(&class_hash)
                .ok_or_else(|| missing("compiled class hash of", class_hash))?;
            layer.compiled_class_hashes.insert(class_hash, *compiled);
        }
        layer.classes.extend(classes);
        Ok(())
    }
}

/// The genesis of a configuration, from which the dumped states are rebuilt.
pub(super) struct Genesis<'a> {
    /// The latest state after genesis.
    pub(super) latest: &'a StarknetState,
    /// The archived state of the genesis block, when blocks are archived.
    pub(super) archived: Option<StarknetState>,
    pub(super) compiled: CompiledClasses<'a>,
}

/// States rebuilt from the genesis.
pub(super) struct RebuiltStates {
    pub(super) latest: StarknetState,
    pub(super) pre_confirmed: StarknetState,
    /// Archived states of accepted blocks, in block order.
    pub(super) archive: Vec<(Felt, StarknetState)>,
}

/// Rebuilds the latest, pre-confirmed and archived states by applying the state diffs of `blocks`
/// and `pre_confirmed_state_diff` to the genesis.
pub(super) fn rebuild_states(
    mut genesis: Genesis,
    blocks: &StarknetBlocks,
    pre_confirmed_state_diff: &StateDiff,
    latest: LatestDump,
) -> DevnetResult<RebuiltStates> {
    let mut accepted = blocks.num_to_hash.iter();
    let (_, genesis_hash) =
        accepted.next().ok_or_else(|| invalid_dump("there is no genesis block"))?;

    let mut archive = Vec::new();
    let mut layer = match genesis.archived.take() {
        Some(archived) => {
            let layer = Layer::of(&archived);
            archive.push((*genesis_hash, archived));
            layer
        }
        None => Layer::of(genesis.latest),
    };
    let archived = !archive.is_empty();

    for (block_number, block_hash) in accepted {
        let state_diff = blocks
            .hash_to_state_diff
            .get(block_hash)
            .ok_or_else(|| invalid_dump(format!("block {block_hash:#x} has no state diff")))?;
        layer.apply_diff(state_diff, block_number.0);
        if archived {
            archive.push((*block_hash, layer.to_state(genesis.latest, &mut genesis.compiled)?));
        }
    }

    let mut pre_confirmed = layer.clone();
    pre_confirmed.apply_diff(pre_confirmed_state_diff, blocks.pre_confirmed_block.block_number().0);
    latest.apply(&mut layer, pre_confirmed_state_diff)?;

    Ok(RebuiltStates {
        latest: layer.to_state(genesis.latest, &mut genesis.compiled)?,
        pre_confirmed: pre_confirmed.to_state(genesis.latest, &mut genesis.compiled)?,
        archive,
    })
}

/// Identifies the genesis state: its layer and the classes declared at genesis.
pub(in crate::starknet) fn genesis_fingerprint(
    state: &StarknetState,
    classes: &CommittedClassStorage,
) -> Felt {
    let Layer {
        storage,
        nonces,
        class_hashes,
        compiled_class_hashes,
        classes: available,
        storage_metadata,
    } = Layer::of(state);
    let CommittedClassStorage { staging, committed, block_number_to_classes: _ } = classes;

    let mut felts = vec![Felt::from(storage.len())];
    felts.extend(storage.into_iter().flat_map(|((address, key), value)| [address, key, value]));
    for map in [nonces, class_hashes, compiled_class_hashes] {
        felts.push(Felt::from(map.len()));
        felts.extend(map.into_iter().flat_map(|(key, value)| [key, value]));
    }
    felts.push(Felt::from(available.len()));
    felts.extend(available);
    felts.push(Felt::from(storage_metadata.len()));
    felts.extend(
        storage_metadata
            .into_iter()
            .flat_map(|((address, key), block)| [address, key, Felt::from(block)]),
    );
    let declared = sorted(
        committed
            .iter()
            .map(|(class_hash, (_, block))| (*class_hash, Felt::from(*block)))
            .chain(staging.keys().map(|class_hash| (*class_hash, Felt::MAX))),
    );
    felts.push(Felt::from(declared.len()));
    felts.extend(declared.into_iter().flat_map(|(class_hash, block)| [class_hash, block]));

    Poseidon::hash_array(&felts)
}

/// A complete state layer with ordered entries.
#[derive(Clone, Default)]
struct Layer {
    storage: BTreeMap<(Felt, Felt), Felt>,
    nonces: BTreeMap<Felt, Felt>,
    class_hashes: BTreeMap<Felt, Felt>,
    compiled_class_hashes: BTreeMap<Felt, Felt>,
    classes: BTreeSet<Felt>,
    storage_metadata: BTreeMap<(Felt, Felt), u64>,
}

impl Layer {
    /// Returns the committed layer of `state`. Every state-changing operation commits its writes,
    /// so the cache over the layer holds no writes between operations.
    fn of(state: &StarknetState) -> Self {
        let StarknetState { state: _, rpc_contract_classes: _, historic_state, storage_metadata } =
            state;
        let DictState {
            storage_view,
            address_to_nonce,
            address_to_class_hash,
            class_hash_to_class,
            class_hash_to_compiled_class_hash,
            defaulter: _,
        } = historic_state;

        Self {
            storage: storage_view
                .iter()
                .map(|((address, key), value)| ((*address.0.key(), *key.0.key()), *value))
                .collect(),
            nonces: address_to_nonce
                .iter()
                .map(|(address, nonce)| (*address.0.key(), nonce.0))
                .collect(),
            class_hashes: address_to_class_hash
                .iter()
                .map(|(address, class_hash)| (*address.0.key(), class_hash.0))
                .collect(),
            compiled_class_hashes: class_hash_to_compiled_class_hash
                .iter()
                .map(|(class_hash, compiled)| (class_hash.0, compiled.0))
                .collect(),
            classes: class_hash_to_class.keys().map(|class_hash| class_hash.0).collect(),
            storage_metadata: storage_metadata
                .iter()
                .map(|((address, key), block)| (((*address).into(), key.to_felt()), *block))
                .collect(),
        }
    }

    /// Applies a state diff committed at `block_number`, as `StarknetState::commit_diff` does.
    fn apply_diff(&mut self, state_diff: &StateDiff, block_number: u64) {
        let StateDiff {
            storage_updates,
            address_to_nonce,
            address_to_class_hash,
            class_hash_to_compiled_class_hash,
            declared_contracts,
            cairo_0_declared_contracts,
            // Also recorded in `address_to_class_hash`.
            replaced_classes: _,
        } = state_diff;

        for (address, updates) in storage_updates {
            for (key, value) in updates {
                let slot = (Felt::from(*address), key.to_felt());
                self.storage.insert(slot, *value);
                self.storage_metadata.insert(slot, block_number);
            }
        }
        self.nonces
            .extend(address_to_nonce.iter().map(|(address, nonce)| ((*address).into(), *nonce)));
        self.class_hashes.extend(
            address_to_class_hash
                .iter()
                .map(|(address, class_hash)| ((*address).into(), *class_hash)),
        );
        self.compiled_class_hashes.extend(class_hash_to_compiled_class_hash);
        self.classes.extend(declared_contracts.iter().chain(cairo_0_declared_contracts));
    }

    /// Builds a state with the defaulter and class storage of `template`.
    fn to_state(
        &self,
        template: &StarknetState,
        compiled: &mut CompiledClasses,
    ) -> DevnetResult<StarknetState> {
        let mut dict_state = DictState::new(template.historic_state.defaulter.clone());
        for ((address, key), value) in &self.storage {
            dict_state
                .storage_view
                .insert((api_address(*address)?, api_storage_key(*key)?), *value);
        }
        for (address, nonce) in &self.nonces {
            dict_state.address_to_nonce.insert(api_address(*address)?, Nonce(*nonce));
        }
        for (address, class_hash) in &self.class_hashes {
            dict_state.address_to_class_hash.insert(api_address(*address)?, ClassHash(*class_hash));
        }
        for (class_hash, compiled_class_hash) in &self.compiled_class_hashes {
            dict_state
                .class_hash_to_compiled_class_hash
                .insert(ClassHash(*class_hash), CompiledClassHash(*compiled_class_hash));
        }
        for class_hash in &self.classes {
            dict_state
                .class_hash_to_class
                .insert(ClassHash(*class_hash), compiled.get(*class_hash)?);
        }

        let mut storage_metadata = HashMap::with_capacity(self.storage_metadata.len());
        for ((address, key), block) in &self.storage_metadata {
            storage_metadata
                .insert((ContractAddress::new(*address)?, StorageKey::new(*key)?), *block);
        }

        Ok(StarknetState {
            state: CachedState::new(dict_state.clone()),
            rpc_contract_classes: template.rpc_contract_classes.clone(),
            historic_state: dict_state,
            storage_metadata,
        })
    }
}

/// Returns the complete layer of `state`, for comparing states in tests.
#[cfg(test)]
pub(super) fn layer_json(state: &StarknetState) -> serde_json::Value {
    let Layer { storage, nonces, class_hashes, compiled_class_hashes, classes, storage_metadata } =
        Layer::of(state);
    serde_json::json!({
        "storage": storage.into_iter().collect::<Vec<_>>(),
        "nonces": nonces.into_iter().collect::<Vec<_>>(),
        "class_hashes": class_hashes.into_iter().collect::<Vec<_>>(),
        "compiled_class_hashes": compiled_class_hashes.into_iter().collect::<Vec<_>>(),
        "classes": classes,
        "storage_metadata": storage_metadata.into_iter().collect::<Vec<_>>(),
    })
}

pub(super) fn sorted<T: Ord>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut items: Vec<T> = items.into_iter().collect();
    items.sort_unstable();
    items
}

fn api_address(address: Felt) -> DevnetResult<starknet_api::core::ContractAddress> {
    Ok(starknet_api::core::ContractAddress::try_from(address)?)
}

fn api_storage_key(key: Felt) -> DevnetResult<starknet_api::state::StorageKey> {
    Ok(starknet_api::state::StorageKey::try_from(key)?)
}

pub(super) fn invalid_dump(msg: impl Into<String>) -> Error {
    Error::InvalidDump(msg.into())
}
