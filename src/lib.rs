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
#[derive(Debug, PartialEq, Eq, Clone, Reject, Serial, SchemaType)]
enum BidError {
    OnlyAccount,               // contracts cant bid
    BidMore,                   // only higher bids accepted, raised when amount is low
    BidTooLate,                // raised when auction ends if someone tries to bid
    AuctionFinalizedButBidded, // Auction finalized but someone tries to bid
}

// finalize function errors
#[derive(Debug, PartialEq, Eq, Clone, Reject, Serial, SchemaType)]
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
#[receive(contract = "auction", name = "bid", payable, mutable)]
fn auction_bid<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State, StateApiType = S>,
    amount: Amount,
) -> Result<(), BidError> {
    // first ensure auction continue
    ensure_eq!(
        host.state_mut().auction_state,
        AuctionState::Continue,
        BidError::AuctionFinalizedButBidded
    );
    // not filter user, everybody can bid maybe except for the owner
    // ensure_eq!(ctx.sender, ctx.owner, BidError::Blacklisted);

    // check time when bid arrives and auction still continue
    let slot_time = ctx.metadata().slot_time();

    ensure!(slot_time <= host.state_mut().end, BidError::BidTooLate);

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

// finalize the auction, send the highest bid to the contract owner
// of the contract instance. In the next version there will be NFT transfer
// to the highest bidder.

#[receive(contract = "auction", name = "finalize", mutable)]
fn auction_finalize<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State, StateApiType = S>,
) -> Result<(), FinalizeError> {
    let state = host.state();
    // ensure auction still continues

    ensure_eq!(
        state.auction_state,
        AuctionState::Continue,
        FinalizeError::AuctionAlreadyFinalized
    );

    let slot_time = ctx.metadata().slot_time();
    // Ensure the auction has ended already
    ensure!(slot_time > state.end, FinalizeError::AuctionStillActive);

    if let Some(account_address) = state.highest_bidder {
        // mark the auction end
        host.state_mut().auction_state = AuctionState::Sold(account_address);
        let owner = ctx.owner();

        let balance = host.self_balance(); // contract balance
        host.invoke_transfer(&owner, balance).unwrap_abort();
    }
    Ok(())
}

#[concordium_cfg_test]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};
    use test_infrastructure::*;

    // a counter for generating new accounts
    static ADDRESS_COUNTER: AtomicU8 = AtomicU8::new(0);
    const AUCTION_END: u64 = 1;
    const ITEM: &str = "Starry night by Van Gogh";

    fn expect_error<E, T>(expr: Result<T, E>, err: E, msg: &str)
    where
        E: Eq + Debug,
        T: Debug,
    {
        let actual = expr.expect_err_report(msg);
        claim_eq!(actual, err)
    }

    fn item_and_param() -> InitParameter {
        InitParameter {
            item: ITEM.into(),
            end: Timestamp::from_timestamp_millis(AUCTION_END),
        }
    }

    fn create_parameter_bytes(parameter: &InitParameter) -> Vec<u8> {
        to_bytes(parameter)
    }

    fn parametrized_init_ctx(parameter_bytes: &[u8]) -> TestInitContext {
        let mut ctx = TestInitContext::empty();
        ctx.set_parameter(parameter_bytes);
        ctx
    }

    fn new_account() -> AccountAddress {
        let account = AccountAddress([ADDRESS_COUNTER.load(Ordering::SeqCst); 32]);
        ADDRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        account
    }

    fn new_account_ctx<'a>() -> (AccountAddress, TestReceiveContext<'a>) {
        let account = new_account();
        let ctx = new_ctx(account, account, AUCTION_END);
        (account, ctx)
    }

    fn new_ctx<'a>(
        owner: AccountAddress,
        sender: AccountAddress,
        slot_time: u64,
    ) -> TestReceiveContext<'a> {
        let mut ctx = TestReceiveContext::empty();
        ctx.set_sender(Address::Account(sender));
        ctx.set_owner(owner);
        ctx.set_metadata_slot_time(Timestamp::from_timestamp_millis(slot_time));
        ctx
    }

    fn bid(
        host: &mut TestHost<State>,
        ctx: &TestContext<TestReceiveOnlyData>,
        amount: Amount,
        current_contract_balance: Amount,
    ) {
        //set balance
        // initial + bid
        host.set_self_balance(amount + current_contract_balance);

        auction_bid(ctx, host, amount).expect_report("Bidding should pass");
    }

    #[concordium_test]
    fn test_init() {
        let parameter_bytes = create_parameter_bytes(&item_and_param());
        let ctx = parametrized_init_ctx(&parameter_bytes);
        let mut state_builder = TestStateBuilder::new();
        let state_result = auction_init(&ctx, &mut state_builder);
        state_result.expect_report("Contract initialize error");
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn it_works() {
//         let result = add(2, 2);
//         assert_eq!(result, 4);
//     }
// }
