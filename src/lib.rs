//! # Implementation of an auction smart contract
//!
//! Accounts can invoke the bid function to participate in the auction.
//! An account has to send some CCD when invoking the bid function.
//! This CCD amount has to exceed the current highest bid to be accepted by the
//! smart contract.
//!
//! The smart contract keeps track of the current highest bidder as well as
//! the CCD amount of the highest bid. The CCD balance of the smart contract
//! represents the highest bid. When a new highest bid is accepted by the smart
//! contract, the smart contract refunds the old highest bidder.
//!
//! Bids have to be placed before the auction ends. The participant with the
//! highest bid (the last bidder) wins the auction.
//!
//! After the auction ends, any account can finalize the auction. The owner of
//! the smart contract instance receives the highest bid (the balance of this
//! contract) when the auction is finalized. This can be done only once.
//!
//! Terminology: `Accounts` are derived from a public/private key pair.
//! `Contract` instances are created by deploying a smart contract
//! module and initializing it.

use concordium_std::*;
use core::fmt::Debug;

// The state of the auction either done or continues
#[derive(Debug, Serialize, SchemaType, Eq, PartialEq, Clone)]
pub enum AuctionState {
    // still accepting bids
    Continue,
    Sold(AccountAddress), //item has been sold the highest bid's owner
}

// the state of the smart contract
// this state can be viewed by querying the node

#[derive(Debug, Serialize, SchemaType, Clone)]
pub struct State {
    // auction state
    auction_state: AuctionState,
    // highest bid's owner gets the item
    // could be none if noone has bidded yes
    highest_bidder: Option<AccountAddress>,
    //what we are gonna send it back as a item
    item: String,
    // when auction ends
    end: Timestamp,
}

// constructor / init function input struct
#[derive(Serialize, SchemaType)]
struct InitParameter {
    item: String,   //specify while starting the auction
    end: Timestamp, // when auction end
}

// special errors
enum BidError {
    OnlyAccount,               // contracts cant bid
    BidMore,                   // only higher bids accepted, raised when amount is low
    BidTooLate,                // raised when auction ends if someone tries to bid
    AuctionFinalizedButBidded, // Auction finalized but someone tries to bid
}

// finalize function errors
#[derive(Debug, PartialEq, Eq, Clone, Reject, SchemaType)]
enum FinalizeError {
    AuctionStillActive, // raised when owner tries to finalize before it's end time
    AuctionAlreadyFinalized, // raised when trying to finalize already finalized one
}

#[derive(Debug, PartialEq, Eq, Clone, Reject, SchemaType)]
enum BlacklistedBidder {
    Blacklisted, // raised either someone from blacklisted accounts or owner
    Allowed,
}

// contract init function every initialize operation invokes this
// acts like a constructor which returns the contract state
#[init(contract = "auction", parameter = "InitParameter")] //initParam
fn auction_init<S: HasStateApi>(
    _ctx: &impl HasInitContext,
    _state_builder: &mut StateBuilder<S>, //can change the state
) -> InitResult<State> {
    //Get input params
    let param: InitParameter = _ctx.parameter_cursor().get()?; //result error handling
    /// create state of contract
    let state = State {
        auction_state: AuctionState::Continue,
        highest_bidder: None,
        item: param.item,
        end: param.end,
    };
    Ok(state)
}
//receive = accepts input from outside
// contract name, function name to invoke
#[receive(
    contract = "auction",
    name = "bid",
    payable,
    mutable,
    error = "BidError"
)]
fn auction_bid<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State, StateApiType = S>,
    amount: Amount,
) -> Result<(), BidError> {
    // first ensure auction continue
    ensure_eq!(
        host.state_mut().auction_state,
        AuctionState::Continue,
        BidError::AuctionAlreadyFinalized
    );
    // not filter user, everybody can bid maybe except for the owner
    ensure_eq!(ctx.sender, ctx.owner, BlacklistedBidder::Blacklisted);

    // check time when bid arrives and auction still continue
    let slot_time = ctx.metadata().slot_time();

    ensure!(slot_time <= host.state_mut().end(), BidError::BidTooLate);

    // ensure only accounts can bid not contracts
    let sender_address = match ctx.sender() {
        Address::Contract(_) => bail!(BidError::OnlyAccount),
        Address::Account(account_address) => account_address,
    };

    // contract balance
    let balance = host.self_balance();

    let balance_before_latest_bid = balance - amount; //amaount given as parameter

    ensure!(amount > balance_before_latest_bid, BidError::BidMore);

    if let Some(account_address) = host.state_mut().highest_bidder.replace(sender_address) {
        host.invoke_transfer(&account_address, balance_before_latest_bid)
            .unwrap_abort();
    }

    Ok(())
}

// view function

#[receive(contract = "auction", name = "view", return_value = "State")]
fn view<'a, 'b, S: HasStateApi>(
    ctx: &'a impl HasReceiveContext,
    host: &'b impl HasHost<State, StateApiType = S>,
) -> ReceiveResult<&'b State> {
    Ok((host.state()))
}
// view highest bid
#[receive(contract = "auction", name = "viewHighestBid", return_value = "Amount")]
fn view_highest_bid<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &impl HasHost<State, StateApiType = S>,
) -> ReceiveResult<Amount> {
    Ok(host.self_balance())
}

// finalize the auction

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn it_works() {
//         let result = add(2, 2);
//         assert_eq!(result, 4);
//     }
// }
