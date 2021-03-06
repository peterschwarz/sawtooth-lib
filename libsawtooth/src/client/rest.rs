// Copyright 2020 Bitwise IO
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A client for interacting with sawtooth services.

use base64::decode;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::VecDeque;

use crate::client::SawtoothClient;
use crate::client::{
    Batch as ClientBatch, Block as ClientBlock, BlockHeader as ClientBlockHeader,
    Header as ClientHeader, SingleState as ClientSingleState, State as ClientState,
    Transaction as ClientTransaction, TransactionHeader as ClientTransactionHeader,
};

pub use super::error::SawtoothClientError;

/// A client that can be used to interact with sawtooth services. This client handles
/// communication with the REST API.
pub struct RestApiSawtoothClient {
    url: String,
}

impl RestApiSawtoothClient {
    /// Create a new 'RestApiSawtoothClient' with the given base 'url'. The URL should be the bind
    /// endpoint of the sawtooth REST API.
    pub fn new(url: &str) -> Self {
        Self { url: url.into() }
    }
}

/// Implement the sawthooth client trait for the REST API sawtooth client
impl SawtoothClient for RestApiSawtoothClient {
    /// Get the batch with the given batch_id from the current blockchain
    fn get_batch(&self, batch_id: String) -> Result<Option<ClientBatch>, SawtoothClientError> {
        let url = format!("{}/batches/{}", &self.url, &batch_id);
        let error_msg = &format!("unable to get batch {}", batch_id);

        Ok(get::<Single<Batch>>(&url, error_msg)?.map(|singlebatch| singlebatch.data.into()))
    }
    /// List all batches in the current blockchain
    fn list_batches(
        &self,
    ) -> Result<
        Box<dyn Iterator<Item = Result<ClientBatch, SawtoothClientError>>>,
        SawtoothClientError,
    > {
        let url = format!("{}/batches", &self.url);

        Ok(Box::new(PagingIter::new(&url)?.map(
            |item: Result<Batch, _>| item.map(|batch| batch.into()),
        )))
    }
    /// Get the transaction with the given transaction_id from the current blockchain
    fn get_transaction(
        &self,
        transaction_id: String,
    ) -> Result<Option<ClientTransaction>, SawtoothClientError> {
        let url = format!("{}/transactions/{}", &self.url, &transaction_id);
        let error_msg = &format!("unable to get transaction {}", transaction_id);

        Ok(get::<Single<Transaction>>(&url, error_msg)?.map(|singletxn| singletxn.data.into()))
    }
    /// List all transactions in the current blockchain.
    fn list_transactions(
        &self,
    ) -> Result<
        Box<dyn Iterator<Item = Result<ClientTransaction, SawtoothClientError>>>,
        SawtoothClientError,
    > {
        let url = format!("{}/transactions", &self.url);

        Ok(Box::new(PagingIter::new(&url)?.map(
            |item: Result<Transaction, _>| item.map(|txn| txn.into()),
        )))
    }
    /// Get the block with the given block_id from the current blockchain
    fn get_block(&self, block_id: String) -> Result<Option<ClientBlock>, SawtoothClientError> {
        let url = format!("{}/blocks/{}", &self.url, &block_id);
        let error_msg = &format!("unable to get block {}", block_id);

        Ok(get::<Single<Block>>(&url, error_msg)?.map(|singleblock| singleblock.data.into()))
    }
    /// List all blocks in the current blockchain
    fn list_blocks(
        &self,
    ) -> Result<
        Box<dyn Iterator<Item = Result<ClientBlock, SawtoothClientError>>>,
        SawtoothClientError,
    > {
        let url = format!("{}/blocks", &self.url);

        Ok(Box::new(PagingIter::new(&url)?.map(
            |item: Result<Block, _>| item.map(|block| block.into()),
        )))
    }
    /// Get the state entry with the given address from the current blockchain
    fn get_state(&self, address: String) -> Result<Option<ClientSingleState>, SawtoothClientError> {
        let url = format!("{}/state/{}", &self.url, &address);
        let error_msg = &format!("unable to get state at address {}", address);

        Ok(get::<SingleState>(&url, error_msg)?.map(convert_single_state))?.transpose()
    }
    /// List all state entries in the current blockchain
    fn list_states(
        &self,
    ) -> Result<
        Box<dyn Iterator<Item = Result<ClientState, SawtoothClientError>>>,
        SawtoothClientError,
    > {
        let url = format!("{}/state", &self.url);

        Ok(Box::new(
            PagingIter::new(&url)?.map(|item: Result<State, _>| item.map(convert_state)?),
        ))
    }
}

/// used for deserializing single objects returned by the REST API.
fn get<T>(url: &str, error_msg: &str) -> Result<Option<T>, SawtoothClientError>
where
    T: for<'a> serde::de::Deserialize<'a> + Sized,
{
    let request = Client::new().get(url);
    let response = request
        .send()
        .map_err(|err| SawtoothClientError::new_with_source("request failed", err.into()))?;

    if response.status().is_success() {
        let obj: T = response.json().map_err(|err| {
            SawtoothClientError::new_with_source("failed to deserialize response body", err.into())
        })?;
        Ok(Some(obj))
    } else if response.status().as_u16() == 404 {
        Ok(None)
    } else {
        let status = response.status();
        let msg: ErrorResponse = response.json().map_err(|err| {
            SawtoothClientError::new_with_source(
                "failed to deserialize error response body",
                err.into(),
            )
        })?;
        Err(SawtoothClientError::new(&format!(
            "{} {} {}",
            error_msg, status, msg
        )))
    }
}

/// Iterator used for parsing and deserializing data returned by the REST API.
struct PagingIter<T>
where
    T: for<'a> serde::de::Deserialize<'a> + Sized,
{
    next: Option<String>,
    cache: VecDeque<T>,
}

impl<T> PagingIter<T>
where
    T: for<'a> serde::de::Deserialize<'a> + Sized,
{
    /// Create a new 'PagingIter' which will make a call to the REST API and load the initial
    /// cache with the first page of items.
    fn new(url: &str) -> Result<Self, SawtoothClientError> {
        let mut new_iter = Self {
            next: Some(url.to_string()),
            cache: VecDeque::with_capacity(0),
        };
        new_iter.reload_cache()?;
        Ok(new_iter)
    }

    /// If another page of items exists, use the 'next' URL from the current page and
    /// reload the cache with the next page of items.
    fn reload_cache(&mut self) -> Result<(), SawtoothClientError> {
        if let Some(url) = &self.next.take() {
            let request = Client::new().get(url);
            let response = request.send().map_err(|err| {
                SawtoothClientError::new_with_source("request failed", err.into())
            })?;

            let page: Page<T> = response.json().map_err(|err| {
                SawtoothClientError::new_with_source(
                    "failed to deserialize response body",
                    err.into(),
                )
            })?;

            self.cache = page.data.into();

            self.next = page.paging.next.map(String::from);
        }
        Ok(())
    }
}

impl<T> Iterator for PagingIter<T>
where
    T: for<'a> serde::de::Deserialize<'a> + Sized,
{
    type Item = Result<T, SawtoothClientError>;
    /// Return the next item from the cache, if the cache is empty reload it.
    fn next(&mut self) -> Option<Self::Item> {
        if self.cache.is_empty() && self.next.is_some() {
            if let Err(err) = self.reload_cache() {
                return Some(Err(err));
            }
        };
        self.cache.pop_front().map(Ok)
    }
}

/// A struct that represents a page of items, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Page<T: Sized> {
    #[serde(bound(deserialize = "T: Deserialize<'de>"))]
    data: Vec<T>,
    paging: PageInfo,
}

#[derive(Debug, Deserialize)]
struct PageInfo {
    next: Option<String>,
}

/// A struct that represents a single state object, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct SingleState {
    data: String,
    head: String,
}

/// A struct that represents a batch, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Batch {
    header: Header,
    header_signature: String,
    trace: bool,
    transactions: Vec<Transaction>,
}

/// A struct that represents a header in a batch, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Header {
    signer_public_key: String,
    transaction_ids: Vec<String>,
}

/// A struct that represents a transaction, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Transaction {
    header: TransactionHeader,
    header_signature: String,
    payload: String,
}

/// A struct that represents a header in a transaction, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct TransactionHeader {
    batcher_public_key: String,
    dependencies: Vec<String>,
    family_name: String,
    family_version: String,
    inputs: Vec<String>,
    nonce: String,
    outputs: Vec<String>,
    payload_sha512: String,
    signer_public_key: String,
}

/// A struct that represents a block, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Block {
    header: BlockHeader,
    header_signature: String,
    batches: Vec<Batch>,
}

/// A struct that represents a header in a block, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct BlockHeader {
    batch_ids: Vec<String>,
    block_num: String,
    consensus: String,
    previous_block_id: String,
    signer_public_key: String,
    state_root_hash: String,
}

/// A struct that represents a state, used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
pub struct State {
    pub address: String,
    pub data: String,
}

/// A struct that represents the data returned by the REST API when retrieving a single item.
/// Used for deserializing JSON objects.
#[derive(Debug, Deserialize)]
struct Single<T: Sized> {
    #[serde(bound(deserialize = "T: Deserialize<'de>"))]
    data: T,
}

impl Into<ClientBatch> for Batch {
    fn into(self) -> ClientBatch {
        let mut txns = Vec::new();
        for t in self.transactions {
            let new_transaction: ClientTransaction = t.into();
            txns.push(new_transaction);
        }
        ClientBatch {
            header: self.header.into(),
            header_signature: self.header_signature,
            trace: self.trace,
            transactions: txns,
        }
    }
}

impl Into<ClientHeader> for Header {
    fn into(self) -> ClientHeader {
        ClientHeader {
            signer_public_key: self.signer_public_key,
            transaction_ids: self.transaction_ids,
        }
    }
}

impl Into<ClientTransaction> for Transaction {
    fn into(self) -> ClientTransaction {
        ClientTransaction {
            header: self.header.into(),
            header_signature: self.header_signature,
            payload: self.payload,
        }
    }
}

impl Into<ClientTransactionHeader> for TransactionHeader {
    fn into(self) -> ClientTransactionHeader {
        ClientTransactionHeader {
            batcher_public_key: self.batcher_public_key,
            dependencies: self.dependencies,
            family_name: self.family_name,
            family_version: self.family_version,
            inputs: self.inputs,
            nonce: self.nonce,
            outputs: self.outputs,
            payload_sha512: self.payload_sha512,
            signer_public_key: self.signer_public_key,
        }
    }
}

impl Into<ClientBlock> for Block {
    fn into(self) -> ClientBlock {
        let clientbatches = self.batches.into_iter().map(|batch| batch.into()).collect();
        ClientBlock {
            header: self.header.into(),
            header_signature: self.header_signature,
            batches: clientbatches,
        }
    }
}

impl Into<ClientBlockHeader> for BlockHeader {
    fn into(self) -> ClientBlockHeader {
        ClientBlockHeader {
            batch_ids: self.batch_ids,
            block_num: self.block_num,
            consensus: self.consensus,
            previous_block_id: self.previous_block_id,
            signer_public_key: self.signer_public_key,
            state_root_hash: self.state_root_hash,
        }
    }
}

fn convert_state(state: State) -> Result<ClientState, SawtoothClientError> {
    Ok(ClientState {
        address: state.address,
        data: decode(state.data).map_err(|err| {
            SawtoothClientError::new_with_source("failed to decode state data", err.into())
        })?,
    })
}

fn convert_single_state(state: SingleState) -> Result<ClientSingleState, SawtoothClientError> {
    Ok(ClientSingleState {
        data: decode(state.data).map_err(|err| {
            SawtoothClientError::new_with_source("failed to decode state data", err.into())
        })?,
        head: state.head,
    })
}

/// Used for deserializing error responses from the Sawtooth REST API.
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: ErrData,
}

#[derive(Debug, Deserialize)]
struct ErrData {
    message: String,
}

impl std::fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.error.message)
    }
}
