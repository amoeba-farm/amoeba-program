//! Off-chain client for the Ameba futures market (`ameba-futures`).
//!
//! * [`Market`]: a market's addresses and every instruction with its exact
//!   accounts.
//! * [`Book`]: decodes the market account (traders, orders, maker ladders,
//!   claims) and builds proposals: [`Book::propose`] runs the exact package
//!   solver over the complete book, the same arc list the program verifies.
//! * Unit helpers between entry prices, premiums and fine units, and
//!   [`error_name`] for the program's errors.
//! * [`governance`]: binds instructions to the controller's gate epochs.
//! * [`oracle`]: the oracle `Period` layout and readers the markets settle on.
pub mod governance;

/// Oracle `Period` layout and readers, for consumers that decode periods.
pub use ameba_oracle_interface as oracle;

pub use ameba_futures::{
    self as program,
    state::{
        self, Claim, Core, CreditRisk, Header, Ladder, LpPosition, Maker, Order, Pool, Rewards,
        StakePool, StakePosition, Step, Tiers, Trader, Vault, VaultPosition, CREDIT_COUNTED,
        CREDIT_SETTLED, CREDIT_SHORTFALL, ORDER_CLOSE, PRICE_SCALE, STATUS_FAILED, STATUS_OPEN,
        STATUS_SETTLED, UNBOND_SECS, VAULT_MARKETS,
    },
    FuturesError, FuturesInstruction, Knot, ERROR_BASE, MODE_GTC, MODE_IOC, MODE_POST, NO_CLAIM,
};
pub use ameba_futures_math::{
    self as math, codec::Proposal, payoff, Arc, ArcId, Side, Solution, Taker,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const SYSTEM: Pubkey = solana_system_interface::program::ID;
/// SPL Token-2022 (reward mints such as AMBA may be of it).
pub const TOKEN_2022: Pubkey =
    solana_program::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const COMPUTE_BUDGET: Pubkey =
    solana_program::pubkey!("ComputeBudget111111111111111111111111111111");
/// The heap frame `Submit` and `Quote` need (the program's own allocator).
pub const HEAP_FRAME: u32 = ameba_futures::heap::HEAP_MAX as u32;

/// `ComputeBudgetInstruction::RequestHeapFrame`.
pub fn heap_frame(bytes: u32) -> Instruction {
    let mut data = vec![1u8];
    data.extend_from_slice(&bytes.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data,
    }
}

/// `ComputeBudgetInstruction::SetComputeUnitLimit`.
pub fn compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data,
    }
}

/// Error name for a custom program error code from this program.
pub fn error_name(code: u32) -> Option<&'static str> {
    use FuturesError::*;
    let all = [
        InvalidInstruction,
        InvalidAccount,
        Unauthorized,
        NotTradable,
        InvalidShape,
        InvalidPrice,
        InsufficientFunds,
        TraderCapacity,
        OrderCapacity,
        ClaimCapacity,
        MakerCapacity,
        BadProposal,
        Unbalanced,
        NotBestPrice,
        NotMaxFill,
        FifoViolation,
        SearchLimit,
        MinFill,
        WouldCross,
        NotComplement,
        ClaimNotFound,
        OrderNotFound,
        NotSettled,
        AlreadyFinal,
        Overflow,
        Custody,
        Insolvent,
        LimitExceeded,
        NotExpired,
        ClaimReserved,
        VaultCapacity,
        CreditLimit,
        NeedsRecovery,
    ];
    let names = [
        "InvalidInstruction",
        "InvalidAccount",
        "Unauthorized",
        "NotTradable",
        "InvalidShape",
        "InvalidPrice",
        "InsufficientFunds",
        "TraderCapacity",
        "OrderCapacity",
        "ClaimCapacity",
        "MakerCapacity",
        "BadProposal",
        "Unbalanced",
        "NotBestPrice",
        "NotMaxFill",
        "FifoViolation",
        "SearchLimit",
        "MinFill",
        "WouldCross",
        "NotComplement",
        "ClaimNotFound",
        "OrderNotFound",
        "NotSettled",
        "AlreadyFinal",
        "Overflow",
        "Custody",
        "Insolvent",
        "LimitExceeded",
        "NotExpired",
        "ClaimReserved",
        "VaultCapacity",
        "CreditLimit",
        "NeedsRecovery",
    ];
    all.iter()
        .zip(names)
        .find(|(e, _)| ERROR_BASE + **e as u32 == code)
        .map(|(_, n)| n)
}

/// One market's addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Market {
    pub program: Pubkey,
    pub market: Pubkey,
    pub registry: Pubkey,
    pub authority: Pubkey,
    pub vault: Pubkey,
    pub period: Pubkey,
    pub mint: Pubkey,
}

/// What `CreateMarket` needs besides addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateArgs {
    pub product_id: [u8; 32],
    pub expiry_ts: u64,
    pub grid: u16,
    pub k: u64,
    pub step_value: u64,
    pub value_decimals: u8,
    pub ref_value: u64,
    pub trader_cap: u16,
    pub order_cap: u16,
    pub maker_cap: u8,
    pub claim_cap: u32,
}

impl CreateArgs {
    pub fn space(&self) -> usize {
        state::market_len(
            self.grid,
            self.trader_cap,
            self.order_cap,
            self.maker_cap,
            self.claim_cap,
        )
    }
}

/// An order to submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitArgs {
    pub side: Side,
    pub lo: u16,
    pub hi: u16,
    pub lots: u64,
    pub limit: u64,
    pub min_fill: u64,
    pub mode: u8,
    /// `(claim slot, claim seq)` this order closes.
    pub close: Option<(u32, u64)>,
}

impl SubmitArgs {
    pub fn taker(&self) -> Taker {
        Taker {
            side: self.side,
            lo: self.lo,
            hi: self.hi,
            limit: self.limit,
            qty: self.lots,
        }
    }
}

fn ix(program: Pubkey, data: FuturesInstruction, accounts: Vec<AccountMeta>) -> Instruction {
    Instruction {
        program_id: program,
        accounts,
        data: borsh::to_vec(&data).expect("borsh"),
    }
}

impl Market {
    /// Addresses of `market` (its vault is the authority's token account the
    /// caller created; pass it in).
    pub fn derive(
        program: Pubkey,
        market: Pubkey,
        vault: Pubkey,
        period: Pubkey,
        mint: Pubkey,
        product_id: &[u8; 32],
        expiry_ts: u64,
    ) -> Self {
        Market {
            program,
            market,
            registry: state::registry_address(&program, product_id, expiry_ts).0,
            authority: state::authority_address(&program, &market).0,
            vault,
            period,
            mint,
        }
    }

    /// The authority PDA of a market (owner of its vault).
    pub fn authority_of(program: &Pubkey, market: &Pubkey) -> Pubkey {
        state::authority_address(program, market).0
    }

    /// `CreateMarket`, after the market account exists (zeroed, owned by the
    /// program, `args.space()` bytes) and the vault is the authority's.
    pub fn create(&self, creator: &Pubkey, oracle_program: &Pubkey, a: &CreateArgs) -> Instruction {
        let program_data = Pubkey::find_program_address(
            &[self.program.as_ref()],
            &solana_sdk_ids::bpf_loader_upgradeable::ID,
        )
        .0;
        ix(
            self.program,
            FuturesInstruction::CreateMarket {
                product_id: a.product_id,
                expiry_ts: a.expiry_ts,
                grid: a.grid,
                k: a.k,
                step_value: a.step_value,
                value_decimals: a.value_decimals,
                ref_value: a.ref_value,
                trader_cap: a.trader_cap,
                order_cap: a.order_cap,
                maker_cap: a.maker_cap,
                claim_cap: a.claim_cap,
            },
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new(self.registry, false),
                AccountMeta::new_readonly(self.period, false),
                AccountMeta::new_readonly(*oracle_program, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(self.vault, false),
                AccountMeta::new(*creator, true),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(self.program, false),
                AccountMeta::new_readonly(program_data, false),
            ],
        )
    }

    pub fn deposit(&self, owner: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Deposit { amount },
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new(*owner_token, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
        )
    }

    /// `u64::MAX` withdraws all free cash.
    pub fn withdraw(&self, owner: &Pubkey, owner_token: &Pubkey, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Withdraw { amount },
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new(*owner_token, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(self.authority, false),
            ],
        )
    }

    /// `Submit`; send it with [`heap_frame`] and a 1.4M [`compute_limit`].
    pub fn submit(&self, owner: &Pubkey, a: &SubmitArgs, proposal: Vec<u8>) -> Instruction {
        let (close_claim, close_seq) = a.close.unwrap_or((NO_CLAIM, 0));
        ix(
            self.program,
            FuturesInstruction::Submit {
                side: a.side as u8,
                lo: a.lo,
                hi: a.hi,
                lots: a.lots,
                limit: a.limit,
                min_fill: a.min_fill,
                mode: a.mode,
                close_claim,
                close_seq,
                proposal,
            },
            self.trading_accounts(owner),
        )
    }

    fn trading_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.market, false),
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new_readonly(self.period, false),
        ]
    }

    fn owner_accounts(&self, owner: &Pubkey) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.market, false),
            AccountMeta::new_readonly(*owner, true),
        ]
    }

    pub fn cancel(&self, owner: &Pubkey, slot: u16, seq: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Cancel { slot, seq },
            self.owner_accounts(owner),
        )
    }

    /// `Quote`; send it with [`heap_frame`].
    pub fn quote(
        &self,
        owner: &Pubkey,
        l: &Ladder,
        knots: Vec<Knot>,
        proposal: Vec<u8>,
    ) -> Instruction {
        self.quote_core(owner, l, l.full(), knots, proposal)
    }

    /// A ladder whose steps outside `core` quote only their last tier.
    pub fn quote_core(
        &self,
        owner: &Pubkey,
        l: &Ladder,
        core: Core,
        knots: Vec<Knot>,
        proposal: Vec<u8>,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Quote {
                lo: l.lo,
                hi: l.hi,
                knots,
                bid_spread: l.bid_spread,
                ask_spread: l.ask_spread,
                bid_lots: l.bid_lots,
                ask_lots: l.ask_lots,
                keep: l.keep,
                tiers: l.tiers.count,
                tier_ratio: l.tiers.ratio,
                tier_spread: l.tiers.spread,
                core_lo: core.lo,
                core_hi: core.hi,
                proposal,
            },
            self.trading_accounts(owner),
        )
    }

    pub fn cancel_quote(&self, owner: &Pubkey) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CancelQuote,
            self.owner_accounts(owner),
        )
    }

    pub fn split(&self, owner: &Pubkey, claim: (u32, u64), at: u16) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Split {
                claim: claim.0,
                seq: claim.1,
                at,
            },
            self.owner_accounts(owner),
        )
    }

    pub fn merge(&self, owner: &Pubkey, keep: (u32, u64), absorb: (u32, u64)) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Merge {
                keep: keep.0,
                keep_seq: keep.1,
                absorb: absorb.0,
                absorb_seq: absorb.1,
            },
            self.owner_accounts(owner),
        )
    }

    pub fn redeem(
        &self,
        owner: &Pubkey,
        long: (u32, u64),
        short: (u32, u64),
        lots: u64,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Redeem {
                long: long.0,
                long_seq: long.1,
                short: short.0,
                short_seq: short.1,
                lots,
            },
            self.owner_accounts(owner),
        )
    }

    pub fn transfer(
        &self,
        owner: &Pubkey,
        claim: (u32, u64),
        lots: u64,
        to: &Pubkey,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Transfer {
                claim: claim.0,
                seq: claim.1,
                lots,
                to: *to,
            },
            self.owner_accounts(owner),
        )
    }

    /// The Earn pool `operator` runs in this market.
    pub fn pool(&self, operator: &Pubkey) -> Pubkey {
        state::pool_address(&self.program, &self.market, operator).0
    }

    /// `owner`'s position account in `pool`.
    pub fn lp(&self, pool: &Pubkey, owner: &Pubkey) -> Pubkey {
        state::lp_address(&self.program, pool, owner).0
    }

    /// `PoolMigrate` (dev builds): moves the legacy pool `holder` owns
    /// entirely into `vault`; both it and the operator sign, `payer` funds
    /// the holder's vault position the first time. `markets`: the vault's.
    pub fn pool_migrate(
        &self,
        pool: &Pubkey,
        holder: &Pubkey,
        vault: &EarnVault,
        payer: &Pubkey,
        markets: &[Pubkey],
    ) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new(self.market, false),
            AccountMeta::new(*pool, false),
            AccountMeta::new(self.lp(pool, holder), false),
            AccountMeta::new(vault.vault, false),
            AccountMeta::new(vault.position(holder), false),
            AccountMeta::new_readonly(*holder, true),
            AccountMeta::new_readonly(vault.operator, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(SYSTEM, false),
        ];
        accounts.extend(markets.iter().map(|m| AccountMeta::new_readonly(*m, false)));
        ix(self.program, FuturesInstruction::PoolMigrate, accounts)
    }

    /// `PoolWithdraw` (legacy pools' exit, dev builds); `u64::MAX` redeems every share.
    pub fn pool_withdraw(
        &self,
        pool: &Pubkey,
        owner: &Pubkey,
        owner_token: &Pubkey,
        shares: u64,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::PoolWithdraw { shares },
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new(*pool, false),
                AccountMeta::new(self.lp(pool, owner), false),
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new(*owner_token, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(self.authority, false),
            ],
        )
    }

    pub fn finalize(&self) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Finalize,
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new_readonly(self.period, false),
            ],
        )
    }

    /// `CouncilSettle` at `value` (oracle units at the market's decimals),
    /// signed by `authority`. On a governed program, add the council with
    /// `governance::with_council_authorization`.
    pub fn council_settle(&self, authority: &Pubkey, value: u64) -> Instruction {
        let program_data = Pubkey::find_program_address(
            &[self.program.as_ref()],
            &solana_sdk_ids::bpf_loader_upgradeable::ID,
        )
        .0;
        ix(
            self.program,
            FuturesInstruction::CouncilSettle { value },
            vec![
                AccountMeta::new(self.market, false),
                AccountMeta::new_readonly(self.period, false),
                AccountMeta::new_readonly(*authority, true),
                AccountMeta::new_readonly(self.program, false),
                AccountMeta::new_readonly(program_data, false),
            ],
        )
    }

    pub fn settle(&self, claims: Vec<u32>) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Settle { claims },
            vec![AccountMeta::new(self.market, false)],
        )
    }

    pub fn settle_maker(&self, maker: u8) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::SettleMaker { maker },
            vec![AccountMeta::new(self.market, false)],
        )
    }

    pub fn prune(&self, orders: Vec<u16>) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Prune { orders },
            vec![AccountMeta::new(self.market, false)],
        )
    }
}

/// The Earn vault's addresses: one vault for the whole product, run by
/// `operator`, its idle cash in `cash`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EarnVault {
    pub program: Pubkey,
    pub vault: Pubkey,
    pub operator: Pubkey,
    pub cash: Pubkey,
    pub mint: Pubkey,
}

impl EarnVault {
    /// The vault `operator` runs, with `cash` as its token account.
    pub fn new(program: Pubkey, operator: Pubkey, cash: Pubkey, mint: Pubkey) -> Self {
        EarnVault {
            program,
            vault: Self::address(&program, &operator),
            operator,
            cash,
            mint,
        }
    }

    /// From the vault account itself.
    pub fn of(program: Pubkey, v: &Vault) -> Self {
        Self::new(program, v.operator, v.cash_account, v.quote_mint)
    }

    /// The vault `operator` runs.
    pub fn address(program: &Pubkey, operator: &Pubkey) -> Pubkey {
        state::vault_address(program, operator).0
    }

    /// `owner`'s position account.
    pub fn position(&self, owner: &Pubkey) -> Pubkey {
        state::vault_lp_address(&self.program, &self.vault, owner).0
    }

    /// `CreateVault`, once `cash` (a token account of `mint` owned by the
    /// vault's address) exists; the operator signs, `payer` funds the rent.
    pub fn create(&self, payer: &Pubkey) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CreateVault,
            vec![
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(self.cash, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        )
    }

    /// `VaultDeposit` of `amount`; `markets` are the vault's, in its order
    /// (`Vault::markets`); `payer` funds the position the first time.
    pub fn deposit(
        &self,
        owner: &Pubkey,
        owner_token: &Pubkey,
        amount: u64,
        payer: &Pubkey,
        markets: &[Pubkey],
    ) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new(self.vault, false),
            AccountMeta::new(self.position(owner), false),
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(*owner_token, false),
            AccountMeta::new(self.cash, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new(*payer, true),
        ];
        accounts.extend(markets.iter().map(|m| AccountMeta::new_readonly(*m, false)));
        ix(
            self.program,
            FuturesInstruction::VaultDeposit { amount },
            accounts,
        )
    }

    /// `VaultWithdraw` of `shares` (`u64::MAX`: all); `markets` are the
    /// vault's, in its order; `draws` the markets (from `markets`) whose free
    /// cash may pay what idle cash does not, in that order.
    pub fn withdraw(
        &self,
        owner: &Pubkey,
        owner_token: &Pubkey,
        shares: u64,
        markets: &[Pubkey],
        draws: &[Market],
    ) -> Instruction {
        let drawn = |m: &Pubkey| draws.iter().any(|d| d.market == *m);
        let mut accounts = vec![
            AccountMeta::new(self.vault, false),
            AccountMeta::new(self.position(owner), false),
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(*owner_token, false),
            AccountMeta::new(self.cash, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ];
        accounts.extend(markets.iter().map(|m| {
            if drawn(m) {
                AccountMeta::new(*m, false)
            } else {
                AccountMeta::new_readonly(*m, false)
            }
        }));
        for d in draws {
            accounts.push(AccountMeta::new(d.vault, false));
            accounts.push(AccountMeta::new_readonly(d.authority, false));
        }
        ix(
            self.program,
            FuturesInstruction::VaultWithdraw { shares },
            accounts,
        )
    }

    /// `VaultFund`: the operator moves `amount` of idle cash into `m`.
    pub fn fund(&self, m: &Market, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::VaultFund { amount },
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new(self.cash, false),
                AccountMeta::new(m.vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }

    /// `VaultDefund`: the operator moves `amount` (`u64::MAX`: all) of the
    /// vault's free cash in `m` back to idle cash.
    pub fn defund(&self, m: &Market, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::VaultDefund { amount },
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.vault, false),
                AccountMeta::new(self.cash, false),
                AccountMeta::new(m.vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(m.authority, false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }
}

impl EarnVault {
    /// `VaultRewards`: the operator sets the reward stream, `amount` atoms
    /// of `mint` from `start_ts` to `end_ts` out of `account` (the vault's
    /// token account of the mint), `house`'s shares left out; `payer` funds
    /// the vault's growth the first time.
    #[allow(clippy::too_many_arguments)]
    pub fn set_rewards(
        &self,
        payer: &Pubkey,
        mint: &Pubkey,
        account: &Pubkey,
        start_ts: u64,
        end_ts: u64,
        amount: u64,
        house: &Pubkey,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::VaultRewards {
                start_ts,
                end_ts,
                amount,
                house: *house,
            },
            vec![
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(*mint, false),
                AccountMeta::new_readonly(*account, false),
                AccountMeta::new_readonly(self.position(house), false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        )
    }

    /// `VaultRewardsClaim` (anyone may send it): pays `owner`'s rewards of
    /// stream `r` into `owner_token`, their token account of the reward mint
    /// under `token_program`.
    pub fn claim_rewards(
        &self,
        r: &Rewards,
        token_program: &Pubkey,
        owner: &Pubkey,
        owner_token: &Pubkey,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::VaultRewardsClaim,
            vec![
                AccountMeta::new(self.vault, false),
                AccountMeta::new(self.position(owner), false),
                AccountMeta::new(r.account, false),
                AccountMeta::new_readonly(r.mint, false),
                AccountMeta::new(*owner_token, false),
                AccountMeta::new_readonly(*token_program, false),
            ],
        )
    }

    /// `VaultRewardsWithdraw`: the operator takes `amount` (`u64::MAX`: all)
    /// of the reward account's surplus into `dest`.
    pub fn withdraw_rewards(
        &self,
        r: &Rewards,
        token_program: &Pubkey,
        dest: &Pubkey,
        amount: u64,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::VaultRewardsWithdraw { amount },
            vec![
                AccountMeta::new(self.vault, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new(r.account, false),
                AccountMeta::new_readonly(r.mint, false),
                AccountMeta::new(*dest, false),
                AccountMeta::new_readonly(*token_program, false),
            ],
        )
    }
}

/// The vault's reward stream accrued to `now`, and what `p` would be paid
/// then (its pending rewards, settled to `now`), as the program counts them;
/// `None` without a stream.
pub fn rewards_at(v: &Vault, p: Option<&VaultPosition>, now: u64) -> Option<(Rewards, u64)> {
    let mut r = v.rewards?;
    r.accrue(v.shares, now)?;
    let pending = match p {
        Some(p) => {
            let mut p = *p;
            r.settle(&mut p)?;
            p.reward_pending
        }
        None => 0,
    };
    Some((r, pending))
}

/// The vault account.
pub fn decode_vault(data: &[u8]) -> Result<Vault, solana_program::program_error::ProgramError> {
    Vault::read(data)
}

/// A vault position account.
pub fn decode_vault_position(
    data: &[u8],
) -> Result<VaultPosition, solana_program::program_error::ProgramError> {
    VaultPosition::read(data)
}

/// `ix` (built for the Earn vault, or a legacy pool, as its owner:
/// `Submit`, `Cancel`, `Quote`, `CancelQuote`, `Split`, `Merge` or `Redeem`)
/// signed by its operator instead: the owner is no signer and the operator
/// signs last.
pub fn operated(mut ix: Instruction, operator: &Pubkey) -> Instruction {
    ix.accounts[1].is_signer = false;
    ix.accounts.push(AccountMeta::new_readonly(*operator, true));
    ix
}

/// A pool account.
pub fn decode_pool(data: &[u8]) -> Result<Pool, solana_program::program_error::ProgramError> {
    Pool::read(data)
}

/// A pool position account.
pub fn decode_lp(data: &[u8]) -> Result<LpPosition, solana_program::program_error::ProgramError> {
    LpPosition::read(data)
}

/// The stake pool behind an Earn vault's maker: its addresses, and every
/// instruction with its exact accounts (`docs/futures.md`, "Staked AMBA
/// backs the maker").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakeAddrs {
    pub program: Pubkey,
    pub pool: Pubkey,
    pub vault: Pubkey,
    pub operator: Pubkey,
    pub amba_mint: Pubkey,
    /// The AMBA mint's token program (Token-2022 for AMBA).
    pub amba_program: Pubkey,
    /// The pool's staked AMBA account and its USDC account.
    pub amba: Pubkey,
    pub usdc: Pubkey,
}

/// The pool's parameters (`CreateStakePool`, `StakeConfig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakeParams {
    pub haircut_bps: u16,
    pub profit_bps: u16,
    pub buffer_bps: u16,
    pub twap_window: u64,
    pub expiry_window: u64,
}

impl StakeParams {
    pub fn of(p: &StakePool) -> Self {
        StakeParams {
            haircut_bps: p.haircut_bps,
            profit_bps: p.profit_bps,
            buffer_bps: p.buffer_bps,
            twap_window: p.twap_window,
            expiry_window: p.expiry_window,
        }
    }
}

impl StakeAddrs {
    /// The pool behind `vault` (run by its operator), its token accounts
    /// `amba` and `usdc` (owned by [`StakeAddrs::address`]).
    pub fn new(
        program: Pubkey,
        vault: &EarnVault,
        amba_mint: Pubkey,
        amba_program: Pubkey,
        amba: Pubkey,
        usdc: Pubkey,
    ) -> Self {
        StakeAddrs {
            program,
            pool: Self::address(&program, &vault.vault),
            vault: vault.vault,
            operator: vault.operator,
            amba_mint,
            amba_program,
            amba,
            usdc,
        }
    }

    /// From the pool account itself.
    pub fn of(program: Pubkey, pool: &Pubkey, p: &StakePool, amba_program: Pubkey) -> Self {
        StakeAddrs {
            program,
            pool: *pool,
            vault: p.vault,
            operator: p.operator,
            amba_mint: p.amba_mint,
            amba_program,
            amba: p.amba_account,
            usdc: p.usdc_account,
        }
    }

    pub fn address(program: &Pubkey, vault: &Pubkey) -> Pubkey {
        state::stake_address(program, vault).0
    }

    pub fn position(&self, owner: &Pubkey) -> Pubkey {
        state::stake_position_address(&self.program, &self.pool, owner).0
    }

    /// `CreateStakePool`: the operator signs, `payer` (the program's upgrade
    /// authority; a governed program adds the council, as for
    /// `CreateMarket`) funds the rent.
    pub fn create(&self, payer: &Pubkey, params: StakeParams) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CreateStakePool {
                haircut_bps: params.haircut_bps,
                profit_bps: params.profit_bps,
                buffer_bps: params.buffer_bps,
                twap_window: params.twap_window,
                expiry_window: params.expiry_window,
            },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.vault, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new_readonly(self.amba, false),
                AccountMeta::new_readonly(self.usdc, false),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(self.program, false),
                AccountMeta::new_readonly(
                    Pubkey::find_program_address(
                        &[self.program.as_ref()],
                        &solana_sdk_ids::bpf_loader_upgradeable::ID,
                    )
                    .0,
                    false,
                ),
            ],
        )
    }

    pub fn config(&self, params: StakeParams) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StakeConfig {
                haircut_bps: params.haircut_bps,
                profit_bps: params.profit_bps,
                buffer_bps: params.buffer_bps,
                twap_window: params.twap_window,
                expiry_window: params.expiry_window,
            },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }

    /// `StakePrice`: USDC atoms per whole AMBA, times `PRICE_SCALE`.
    pub fn post_price(&self, price: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StakePrice { price },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }

    /// `PoolPrice` (permissionless): the AMBA price from mainnet's Meteora
    /// AMBA/SOL pool and Pyth's SOL/USD feed. Send it alone (compute-budget
    /// instructions aside), or the program refuses it.
    pub fn pool_price(&self) -> Instruction {
        use ameba_futures::price_source as ps;
        ix(
            self.program,
            FuturesInstruction::PoolPrice,
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(ps::AMBA_SOL_POOL, false),
                AccountMeta::new_readonly(ps::SOL_USD_FEED, false),
                AccountMeta::new_readonly(solana_program::sysvar::instructions::ID, false),
            ],
        )
    }

    /// `Stake` of `amount` AMBA atoms from `owner_amba`; `payer` funds the
    /// position the first time.
    pub fn stake(
        &self,
        owner: &Pubkey,
        owner_amba: &Pubkey,
        amount: u64,
        payer: &Pubkey,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::Stake { amount },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.position(owner), false),
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new(*owner_amba, false),
                AccountMeta::new(self.amba, false),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new_readonly(self.amba_program, false),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new(*payer, true),
            ],
        )
    }

    /// `Unstake` of `shares` (`u64::MAX`: all) of `owner`'s unstaking shares
    /// whose timer is done ([`StakePool::unbond_ends`]) into `owner_amba`,
    /// the owner signing. `markets`: every market holding the pool's credit
    /// unsettled ([`unstake_markets`]).
    pub fn unstake(
        &self,
        owner: &Pubkey,
        owner_amba: &Pubkey,
        shares: u64,
        markets: &[Pubkey],
    ) -> Instruction {
        self.complete_unstake(owner, owner_amba, shares, owner, markets)
    }

    /// `Unstake` as [`StakeAddrs::unstake`], sent by anyone (`signer`, the
    /// keeper for instance): the AMBA still goes only into the owner's own
    /// account `owner_amba`, and anyone but the owner pays all of the
    /// unstaking shares (`u64::MAX`).
    pub fn complete_unstake(
        &self,
        owner: &Pubkey,
        owner_amba: &Pubkey,
        shares: u64,
        signer: &Pubkey,
        markets: &[Pubkey],
    ) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new(self.pool, false),
            AccountMeta::new(self.position(owner), false),
            AccountMeta::new_readonly(*signer, true),
            AccountMeta::new(*owner_amba, false),
            AccountMeta::new(self.amba, false),
            AccountMeta::new_readonly(self.amba_mint, false),
            AccountMeta::new_readonly(self.amba_program, false),
        ];
        accounts.extend(markets.iter().map(|m| AccountMeta::new_readonly(*m, false)));
        ix(
            self.program,
            FuturesInstruction::Unstake { shares },
            accounts,
        )
    }

    /// `StartUnstake` of `shares` (`u64::MAX`: all not unstaking yet):
    /// `Unstake` pays them `UNBOND_SECS` later; adding more restarts the
    /// timer. `payer` funds an older pool's or position's growth.
    pub fn start_unstake(&self, owner: &Pubkey, shares: u64, payer: &Pubkey) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StartUnstake { shares },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.position(owner), false),
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new(*payer, true),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        )
    }

    /// `CancelUnstake`: `owner`'s unstaking shares are active again.
    pub fn cancel_unstake(&self, owner: &Pubkey) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CancelUnstake,
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.position(owner), false),
                AccountMeta::new_readonly(*owner, true),
            ],
        )
    }

    /// `StakeClaim` (anyone may send it): `owner`'s AMBA stream rewards into
    /// `owner_amba` and USDC into `owner_usdc`. `reward_account` is the
    /// stream's (any account without one: nothing is paid from it).
    pub fn claim(
        &self,
        owner: &Pubkey,
        reward_account: &Pubkey,
        owner_amba: &Pubkey,
        owner_usdc: &Pubkey,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StakeClaim,
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.position(owner), false),
                AccountMeta::new(*reward_account, false),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new(*owner_amba, false),
                AccountMeta::new_readonly(self.amba_program, false),
                AccountMeta::new(self.usdc, false),
                AccountMeta::new(*owner_usdc, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
        )
    }

    /// `StakeRewards`: the operator sets the AMBA subsidy stream out of
    /// `account` (an AMBA account the pool owns, not its staked one).
    pub fn set_rewards(
        &self,
        account: &Pubkey,
        start_ts: u64,
        end_ts: u64,
        amount: u64,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StakeRewards {
                start_ts,
                end_ts,
                amount,
            },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new_readonly(*account, false),
            ],
        )
    }

    /// `StakeRewardsWithdraw`: the operator takes `amount` (`u64::MAX`: all)
    /// of the stream account's surplus into `dest`.
    pub fn withdraw_rewards(&self, account: &Pubkey, dest: &Pubkey, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::StakeRewardsWithdraw { amount },
            vec![
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
                AccountMeta::new(*account, false),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new(*dest, false),
                AccountMeta::new_readonly(self.amba_program, false),
            ],
        )
    }

    /// `CreditFund`: the operator lends `amount` more credit in `m`.
    pub fn credit_fund(&self, m: &Market, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CreditFund { amount },
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }

    /// `CreditDefund`: the operator takes back `amount` (`u64::MAX`: all it
    /// can) of `m`'s credit.
    pub fn credit_defund(&self, m: &Market, amount: u64) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CreditDefund { amount },
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.pool, false),
                AccountMeta::new_readonly(self.operator, true),
            ],
        )
    }

    /// `CreditSettle` (anyone, after finalization and the vault's own
    /// settlement there).
    pub fn credit_settle(&self, m: &Market) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::CreditSettle,
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(m.vault, false),
                AccountMeta::new_readonly(m.authority, false),
                AccountMeta::new(self.usdc, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
        )
    }

    /// `SettleRecovery` (anyone): one owner's `claims` (and with `maker`
    /// its ladder inventory) in a shortfall market `m`, the AMBA part into
    /// `owner_amba`, the owner's AMBA account.
    pub fn settle_recovery(
        &self,
        m: &Market,
        owner_amba: &Pubkey,
        claims: Vec<u32>,
        maker: bool,
    ) -> Instruction {
        ix(
            self.program,
            FuturesInstruction::SettleRecovery { claims, maker },
            vec![
                AccountMeta::new(m.market, false),
                AccountMeta::new(self.pool, false),
                AccountMeta::new(self.amba, false),
                AccountMeta::new_readonly(self.amba_mint, false),
                AccountMeta::new(*owner_amba, false),
                AccountMeta::new_readonly(self.amba_program, false),
            ],
        )
    }
}

/// The markets holding a stake pool's credit, as `Unstake` takes them
/// ([`unstake_markets`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnstakeMarkets {
    /// The pool's counted markets among those given
    /// ([`Header::counts_credit_of`]): `Unstake`'s accounts after its fixed
    /// ones.
    pub markets: Vec<Pubkey>,
    /// `Unstake` waits now (`NotSettled`): one of them is in its last
    /// `expiry_window` or past expiry unsettled ([`Header::holds_unstaking`]),
    /// or the pool, from before the count, lends anything
    /// ([`StakePool::counted`]). It waits until the markets it backs settle.
    pub waits: bool,
    /// Every one of the pool's [`StakePool::credit_markets`] is among them
    /// (`Unstake` refuses fewer, `InvalidAccount`).
    pub complete: bool,
}

/// Of `markets` (address and header, any markets: the program's, a
/// [`Book`] list), those `Unstake` takes for pool `p` at `now`, and whether
/// it waits for them (Codex's review of the unstake timer, 2026-10-18).
pub fn unstake_markets(
    p: &StakePool,
    markets: impl IntoIterator<Item = (Pubkey, Header)>,
    now: u64,
) -> UnstakeMarkets {
    let mut out = UnstakeMarkets {
        waits: !p.counted(),
        ..UnstakeMarkets::default()
    };
    for (at, h) in markets {
        if !h.counts_credit_of(&p.vault) || out.markets.contains(&at) {
            continue;
        }
        out.waits |= h.holds_unstaking(p.expiry_window, now);
        out.markets.push(at);
    }
    out.complete = out.markets.len() == p.credit_markets as usize;
    out
}

/// A stake pool account.
pub fn decode_stake_pool(
    data: &[u8],
) -> Result<StakePool, solana_program::program_error::ProgramError> {
    StakePool::read(data)
}

/// A stake position account.
pub fn decode_stake_position(
    data: &[u8],
) -> Result<StakePosition, solana_program::program_error::ProgramError> {
    StakePosition::read(data)
}

/// The pool and `p` as an instruction at `now` would see them: the stream
/// accrued, `p` settled (a position of an earlier share epoch holds no shares
/// then: read its shares and its unstaking shares, `unbonding`, from here,
/// never from the account; [`StakePool::unbond_ends`] says when those may
/// leave).
pub fn stake_position_at(
    pool: &StakePool,
    p: &StakePosition,
    now: u64,
) -> Option<(StakePool, StakePosition)> {
    let (mut pool, mut p) = (*pool, *p);
    pool.touch(&mut p, now)?;
    Some((pool, p))
}

/// The pool brought to `now` and what `p` would be paid then (AMBA stream,
/// USDC), as the program counts them.
pub fn stake_at(
    pool: &StakePool,
    p: Option<&StakePosition>,
    now: u64,
) -> Option<(StakePool, u64, u64)> {
    let mut pool = *pool;
    match p {
        Some(p) => {
            let mut p = *p;
            pool.touch(&mut p, now)?;
            Some((pool, p.reward_pending, p.usdc_pending))
        }
        None => {
            let shares = pool.shares;
            if let Some(r) = pool.rewards.as_mut() {
                r.accrue(shares, now)?;
            }
            Some((pool, 0, 0))
        }
    }
}

/// A maker slot, its head and every step that quotes or holds inventory.
pub type LadderView = (u8, Maker, Vec<(u16, Step)>);

/// A decoded market account.
#[derive(Clone, Debug)]
pub struct Book {
    pub header: Header,
    pub data: Vec<u8>,
}

/// A solved order: the package, its encoded proposal and the arcs it is over.
#[derive(Clone, Debug)]
pub struct Proposed {
    pub solution: Solution,
    pub proposal: Vec<u8>,
    pub arcs: Vec<Arc>,
}

impl Book {
    pub fn decode(data: &[u8]) -> Result<Self, solana_program::program_error::ProgramError> {
        let header = Header::read(data)?;
        Ok(Book {
            header,
            data: data.to_vec(),
        })
    }

    pub fn traders(&self) -> Vec<(u16, Trader)> {
        (0..self.header.trader_cap)
            .map(|i| (i, Trader::read(&self.data, self.header.trader_off(i))))
            .filter(|(_, t)| t.owner != Pubkey::default())
            .collect()
    }

    pub fn trader_of(&self, owner: &Pubkey) -> Option<(u16, Trader)> {
        self.traders().into_iter().find(|(_, t)| t.owner == *owner)
    }

    pub fn orders(&self) -> Vec<(u16, Order)> {
        (0..self.header.order_cap)
            .map(|i| (i, Order::read(&self.data, self.header.order_off(i))))
            .filter(|(_, o)| o.seq != 0)
            .collect()
    }

    /// Ladders in use, with every step that quotes or holds inventory.
    pub fn makers(&self) -> Vec<LadderView> {
        let h = &self.header;
        (0..h.maker_cap)
            .filter_map(|m| {
                let mk = Maker::read(&self.data, h.maker_off(m));
                if !mk.used {
                    return None;
                }
                let steps = (0..h.grid)
                    .map(|x| (x, Step::read(&self.data, h.step_off(m, x))))
                    .filter(|(_, s)| s.inv != 0 || s.bid_lots > 0 || s.ask_lots > 0)
                    .collect();
                Some((m, mk, steps))
            })
            .collect()
    }

    pub fn claims(&self) -> Vec<(u32, Claim)> {
        (0..self.header.claim_cap)
            .map(|i| (i, Claim::read(&self.data, self.header.claim_off(i))))
            .filter(|(_, c)| c.lots > 0)
            .collect()
    }

    /// What `owner` holds here is worth, `(lowest, highest)` atoms over every
    /// settlement outcome (the program's own pool valuation); zero if absent.
    /// For the market's credit owner (an Earn vault its stake pool lent
    /// credit) this is the raw value, its debt still in it:
    /// [`Book::vault_value_at`] is what the vault's shares price.
    pub fn value_of(&self, owner: &Pubkey) -> (u64, u64) {
        self.trader_of(owner)
            .and_then(|(t, _)| state::trader_value(&self.data, &self.header, t))
            .unwrap_or((0, 0))
    }

    /// What the Earn vault `vault` holds here is worth to its shares at
    /// `now`, as `VaultDeposit` and `VaultWithdraw` count it: with credit,
    /// its equity (less the debt, never below zero, less the stakers' share
    /// of the profit; `state::credit_equity`).
    pub fn vault_value_at(&self, vault: &Pubkey, now: u64) -> (u64, u64) {
        let (lo, hi) = self.value_of(vault);
        if self.header.credit_owner != *vault {
            return (lo, hi);
        }
        (
            state::credit_equity(&self.header, lo, now),
            state::credit_equity(&self.header, hi, now),
        )
    }

    /// What the credit owner's position here could cost its stakers at
    /// worst over every fill of its resting liquidity, the bound the
    /// program keeps within the credit limit (`state::credit_exposure`);
    /// zero without credit or a slot.
    pub fn credit_exposure(&self) -> u64 {
        let h = &self.header;
        if !h.has_credit() {
            return 0;
        }
        self.trader_of(&h.credit_owner)
            .and_then(|(t, _)| state::credit_exposure(&self.data, h, t))
            .unwrap_or(0)
    }

    /// The credit owner's exposure and what carries its debt (its own cash
    /// and the backing its claims and inventory hold), now or (`resting`)
    /// at worst over every fill of its resting liquidity, as the program
    /// counts them (`state::credit_risk`). `None` without credit or a slot.
    pub fn credit_risk(&self, resting: bool) -> Option<CreditRisk> {
        let h = &self.header;
        if !h.has_credit() {
            return None;
        }
        let (t, _) = self.trader_of(&h.credit_owner)?;
        state::credit_risk(&self.data, h, t, resting)
    }

    /// Whether the market's credit rules admit `owner`'s ladder `l` at `now`
    /// as `Quote` posts it (`state::credit_admits`): the old ladder's escrow
    /// released, the new one funded from its free cash and, as the credit
    /// owner while its market lends, what it borrows; then, at worst over
    /// every fill of its resting liquidity, its exposure within the limit and
    /// its debt carried by its own cash and backing. `false` when that cash
    /// and credit can't fund the ladder (or on overflow); the crossing check
    /// is [`Book::quote_proposal_core`]'s.
    pub fn quote_admits(
        &self,
        owner: &Pubkey,
        l: &Ladder,
        core: Core,
        knots: &[Knot],
        now: u64,
    ) -> bool {
        let Some((t, mut tr)) = self.trader_of(owner) else {
            return false;
        };
        let mut h = self.header;
        let mut data = self.data.clone();
        let m = if tr.maker > 0 {
            tr.maker - 1
        } else {
            let Some(m) = (0..h.maker_cap).find(|&m| !Maker::read(&data, h.maker_off(m)).used)
            else {
                return false;
            };
            Maker {
                used: true,
                trader: t,
                ..Maker::default()
            }
            .write(&mut data, h.maker_off(m));
            tr.maker = m + 1;
            m
        };
        let old = Maker::read(&data, h.maker_off(m)).escrow;
        let (Some(free), Some(reserved)) = (tr.free.checked_add(old), tr.reserved.checked_sub(old))
        else {
            return false;
        };
        let Some(escrow) = state::write_ladder(&mut data, &h, m, l, core, knots) else {
            return false;
        };
        let mut free = free;
        if free < escrow {
            if h.credit_owner != *owner || !self.lends_at(now) {
                return false;
            }
            let Some(debt) = h.debt.checked_add(escrow - free) else {
                return false;
            };
            h.debt = debt;
            free = escrow;
        }
        let Some(reserved) = reserved.checked_add(escrow) else {
            return false;
        };
        tr.free = free - escrow;
        tr.reserved = reserved;
        tr.write(&mut data, h.trader_off(t));
        let mut mk = Maker::read(&data, h.maker_off(m));
        mk.seq = h.next_seq + 1;
        mk.lo = l.lo;
        mk.hi = l.hi;
        mk.escrow = escrow;
        mk.write(&mut data, h.maker_off(m));
        state::credit_admits(&data, &h, t).unwrap_or(false)
    }

    /// Whether the credit owner may borrow here at `now`: open, its credit
    /// not settled, before `credit_until`.
    pub fn lends_at(&self, now: u64) -> bool {
        let h = &self.header;
        h.has_credit() && h.status == STATUS_OPEN && !h.credit_settled() && now < h.credit_until
    }

    /// What the credit owner may still borrow here for new escrow: its
    /// limit less its exposure (0 near expiry, `credit_until`, or once
    /// final).
    pub fn credit_headroom(&self, now: u64) -> u64 {
        if !self.lends_at(now) {
            return 0;
        }
        self.header
            .credit_limit
            .saturating_sub(self.credit_exposure())
    }

    /// What `owner`'s next `Quote` here may escrow beyond its own cash (its
    /// free and reserved cash less its debt) at `now`: as the credit owner
    /// while its market lends, its limit plus the least its claims and
    /// inventory pay, less its debt; the program then keeps its exposure
    /// within the limit. 0 for anyone else.
    pub fn credit_room(&self, owner: &Pubkey, now: u64) -> u64 {
        let h = &self.header;
        if h.credit_owner != *owner || !self.lends_at(now) {
            return 0;
        }
        let Some((t, tr)) = self.trader_of(owner) else {
            return 0;
        };
        let low = state::trader_value(&self.data, h, t).map_or(0, |v| v.0);
        let least = low.saturating_sub(tr.free.saturating_add(tr.reserved));
        h.credit_limit.saturating_add(least).saturating_sub(h.debt)
    }

    /// `owner`'s own cash here: free and reserved, less what the credit
    /// owner borrowed.
    pub fn own_cash(&self, owner: &Pubkey) -> u64 {
        let Some((_, tr)) = self.trader_of(owner) else {
            return 0;
        };
        let cash = tr.free.saturating_add(tr.reserved);
        if self.header.credit_owner == *owner && !self.header.credit_settled() {
            cash.saturating_sub(self.header.debt)
        } else {
            cash
        }
    }

    pub fn claims_of(&self, trader: u16) -> Vec<(u32, Claim)> {
        self.claims()
            .into_iter()
            .filter(|(_, c)| c.trader == trader)
            .collect()
    }

    /// The complete arc list the program verifies against.
    pub fn arcs(&self) -> Vec<Arc> {
        state::book_arcs(&self.data, &self.header)
    }

    /// The package owed to `taker` and its encoded proposal.
    pub fn propose(&self, taker: &Taker) -> Proposed {
        let arcs = self.arcs();
        let solution = math::solve(&arcs, Some(taker), self.header.grid);
        let proposal = Proposal::from_fills(
            &arcs,
            &solution.fills,
            solution.taker_fill,
            &solution.potentials,
        )
        .encode();
        Proposed {
            solution,
            proposal,
            arcs,
        }
    }

    /// The post-only proposal for `owner`'s ladder quote, or `None` if the
    /// ladder would cross the book (some package would fill at once).
    pub fn quote_proposal(&self, owner: &Pubkey, l: &Ladder, knots: &[Knot]) -> Option<Vec<u8>> {
        self.quote_proposal_core(owner, l, l.full(), knots)
    }

    /// The escrow `Quote` reserves for `owner`'s ladder `l` now: what each
    /// step needs beyond what its inventory backs (`Maker::step_escrow`).
    /// `None` on overflow.
    pub fn ladder_escrow(
        &self,
        owner: &Pubkey,
        l: &Ladder,
        core: Core,
        knots: &[Knot],
    ) -> Option<u64> {
        let h = self.header;
        let mut data = self.data.clone();
        let m = match self.trader_of(owner) {
            Some((_, t)) if t.maker > 0 => t.maker - 1,
            _ => (0..h.maker_cap).find(|&m| !Maker::read(&data, h.maker_off(m)).used)?,
        };
        state::write_ladder(&mut data, &h, m, l, core, knots)
    }

    /// Cash a fresh claim of `lots` on `[lo, hi)` (`side`) of trader `t`
    /// releases at once by netting against `t`'s opposite claims, as the
    /// program nets it (`state::plan_net`; ignoring the rare match left out
    /// for want of claim slots).
    pub fn net_release(&self, t: u16, side: Side, lo: u16, hi: u16, lots: u64) -> u128 {
        let opposite: Vec<(u32, Claim)> = self
            .claims_of(t)
            .into_iter()
            .filter(|(_, c)| c.side != side as u8)
            .collect();
        state::plan_net(lo, hi, lots, &opposite)
            .iter()
            .map(|m| u128::from(m.lots) * u128::from(payoff::set_value(m.lo, m.hi, self.header.k)))
            .sum()
    }

    /// As [`Book::quote_proposal`], for [`Market::quote_core`].
    pub fn quote_proposal_core(
        &self,
        owner: &Pubkey,
        l: &Ladder,
        core: Core,
        knots: &[Knot],
    ) -> Option<Vec<u8>> {
        let h = self.header;
        let mut data = self.data.clone();
        let m = match self.trader_of(owner) {
            Some((_, t)) if t.maker > 0 => t.maker - 1,
            _ => (0..h.maker_cap).find(|&m| !Maker::read(&data, h.maker_off(m)).used)?,
        };
        state::write_ladder(&mut data, &h, m, l, core, knots)?;
        let mut mk = Maker::read(&data, h.maker_off(m));
        mk.used = true;
        mk.seq = h.next_seq + 1;
        mk.lo = l.lo;
        mk.hi = l.hi;
        mk.write(&mut data, h.maker_off(m));
        let arcs = state::book_arcs(&data, &h);
        let s = math::solve(&arcs, None, h.grid);
        if s.fills.iter().any(|&f| f > 0) {
            return None;
        }
        Some(
            Proposal {
                fills: vec![],
                taker_fill: 0,
                potentials: s.potentials,
            }
            .encode(),
        )
    }
}

/// Premium per lot for a claim on `[lo, hi)` entered at `entry_fine`
/// (fine units: `step / k`): `entry - lo`, clamped to the claim's range.
pub fn premium_at(lo: u16, hi: u16, entry_fine: u64, k: u64) -> u64 {
    payoff::long_per_lot(lo, hi, entry_fine, k)
}

/// The entry (fine units) a premium per lot implies on `[lo, hi)`.
pub fn entry_of(lo: u16, premium: u64, k: u64) -> u64 {
    u64::from(lo) * k + premium
}

/// A product id is the ASCII symbol, zero padded to 32 bytes.
pub fn product_id(symbol: &str) -> [u8; 32] {
    let mut id = [0u8; 32];
    let b = symbol.as_bytes();
    id[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
    id
}

pub fn product_symbol(id: &[u8; 32]) -> String {
    let end = id.iter().position(|b| *b == 0).unwrap_or(32);
    String::from_utf8_lossy(&id[..end]).into_owned()
}
