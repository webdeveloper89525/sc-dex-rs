#![no_std]

mod cache;
mod events;
mod locked_asset;
pub mod locked_asset_token_merge;

elrond_wasm::imports!();
elrond_wasm::derive_imports!();

const DEFAULT_TRANSFER_EXEC_GAS_LIMIT: u64 = 35000000;
const ADDITIONAL_AMOUNT_TO_CREATE: u64 = 1;
const EPOCHS_IN_MONTH: u64 = 30;

use common_structs::{
    Epoch, LockedAssetTokenAttributes, Nonce, UnlockMilestone, UnlockPeriod, UnlockSchedule,
};

#[elrond_wasm::contract]
pub trait LockedAssetFactory:
    locked_asset::LockedAssetModule
    + cache::CacheModule
    + token_supply::TokenSupplyModule
    + token_send::TokenSendModule
    + token_merge::TokenMergeModule
    + locked_asset_token_merge::LockedAssetTokenMergeModule
    + events::EventsModule
{
    #[init]
    fn init(
        &self,
        asset_token_id: TokenIdentifier,
        #[var_args] default_unlock_period: VarArgs<UnlockMilestone>,
    ) -> SCResult<()> {
        require!(
            asset_token_id.is_valid_esdt_identifier(),
            "Asset token ID is not a valid esdt identifier"
        );
        require!(
            asset_token_id != self.locked_asset_token_id().get(),
            "Asset token ID cannot be the same as Locked asset token ID"
        );
        self.validate_unlock_milestones(&default_unlock_period)?;

        self.transfer_exec_gas_limit()
            .set_if_empty(&DEFAULT_TRANSFER_EXEC_GAS_LIMIT);
        self.init_epoch()
            .set_if_empty(&self.blockchain().get_block_epoch());

        self.asset_token_id().set(&asset_token_id);
        self.default_unlock_period()
            .set(&UnlockPeriod::from(default_unlock_period.into_vec()));
        Ok(())
    }

    #[only_owner]
    #[endpoint]
    fn whitelist(&self, address: ManagedAddress) -> SCResult<()> {
        let is_new = self.whitelisted_contracts().insert(address);
        require!(is_new, "ManagedAddress already whitelisted");
        Ok(())
    }

    #[only_owner]
    #[endpoint(removeWhitelist)]
    fn remove_whitelist(&self, address: ManagedAddress) -> SCResult<()> {
        let is_removed = self.whitelisted_contracts().remove(&address);
        require!(is_removed, "ManagedAddresss not whitelisted");
        Ok(())
    }

    #[endpoint(createAndForwardCustomPeriod)]
    fn create_and_forward_custom_period(
        &self,
        amount: BigUint,
        address: ManagedAddress,
        start_epoch: Epoch,
        unlock_period: UnlockPeriod,
    ) -> SCResult<EsdtTokenPayment<Self::Api>> {
        let caller = self.blockchain().get_caller();
        require!(
            self.whitelisted_contracts().contains(&caller),
            "Permission denied"
        );
        require!(!unlock_period.unlock_milestones.is_empty(), "Empty arg");

        let month_start_epoch = self.get_month_start_epoch(start_epoch);
        let attr = LockedAssetTokenAttributes {
            unlock_schedule: self.create_unlock_schedule(month_start_epoch, unlock_period),
            is_merged: false,
        };

        let new_token =
            self.produce_tokens_and_send(&amount, &attr, &address, &OptionalArg::None)?;

        self.emit_create_and_forward_event(
            &caller,
            &address,
            &new_token.token_identifier,
            new_token.token_nonce,
            &new_token.amount,
            &attr,
            month_start_epoch,
        );
        Ok(new_token)
    }

    #[endpoint(createAndForward)]
    fn create_and_forward(
        &self,
        amount: BigUint,
        address: ManagedAddress,
        start_epoch: Epoch,
        #[var_args] opt_accept_funds_func: OptionalArg<BoxedBytes>,
    ) -> SCResult<EsdtTokenPayment<Self::Api>> {
        let caller = self.blockchain().get_caller();
        require!(
            self.whitelisted_contracts().contains(&caller),
            "Permission denied"
        );
        require!(!self.locked_asset_token_id().is_empty(), "No SFT issued");
        require!(amount > 0, "Zero input amount");
        require!(
            start_epoch >= self.init_epoch().get(),
            "Invalid start epoch"
        );

        let month_start_epoch = self.get_month_start_epoch(start_epoch);
        let unlock_period = self.default_unlock_period().get();
        let attr = LockedAssetTokenAttributes {
            unlock_schedule: self.create_unlock_schedule(month_start_epoch, unlock_period),
            is_merged: false,
        };

        let new_token =
            self.produce_tokens_and_send(&amount, &attr, &address, &opt_accept_funds_func)?;

        self.emit_create_and_forward_event(
            &caller,
            &address,
            &new_token.token_identifier,
            new_token.token_nonce,
            &new_token.amount,
            &attr,
            start_epoch,
        );
        Ok(new_token)
    }

    #[payable("*")]
    #[endpoint(unlockAssets)]
    fn unlock_assets(
        &self,
        #[payment_token] token_id: TokenIdentifier,
        #[payment_amount] amount: BigUint,
        #[payment_nonce] token_nonce: Nonce,
    ) -> SCResult<()> {
        let locked_token_id = self.locked_asset_token_id().get();
        require!(token_id == locked_token_id, "Bad payment token");

        let attributes = self.get_attributes(&token_id, token_nonce)?;
        let unlock_schedule = &attributes.unlock_schedule;

        let month_start_epoch = self.get_month_start_epoch(self.blockchain().get_block_epoch());
        let unlock_amount = self.get_unlock_amount(
            &amount,
            month_start_epoch,
            &unlock_schedule.unlock_milestones,
        );
        require!(amount >= unlock_amount, "Cannot unlock more than locked");
        require!(unlock_amount > 0, "Method called too soon");

        let caller = self.blockchain().get_caller();
        self.mint_and_send_assets(&caller, &unlock_amount);

        let mut output_locked_assets_token_amount = EsdtTokenPayment {
            token_identifier: token_id.clone(),
            token_nonce: 0,
            amount: BigUint::zero(),
            token_type: EsdtTokenType::Invalid,
        };
        let mut output_locked_asset_attributes = LockedAssetTokenAttributes {
            unlock_schedule: UnlockSchedule {
                unlock_milestones: Vec::new(),
            },
            is_merged: false,
        };

        let locked_remaining = &amount - &unlock_amount;
        if locked_remaining > 0 {
            let new_unlock_milestones = self.create_new_unlock_milestones(
                month_start_epoch,
                &unlock_schedule.unlock_milestones,
            );
            output_locked_asset_attributes = LockedAssetTokenAttributes {
                unlock_schedule: UnlockSchedule {
                    unlock_milestones: new_unlock_milestones,
                },
                is_merged: attributes.is_merged,
            };
            output_locked_assets_token_amount = self.produce_tokens_and_send(
                &locked_remaining,
                &output_locked_asset_attributes,
                &caller,
                &OptionalArg::None,
            )?;
        }

        self.nft_burn_tokens(&locked_token_id, token_nonce, &amount);

        self.emit_unlock_assets_event(
            &caller,
            &token_id,
            token_nonce,
            &amount,
            &output_locked_assets_token_amount.token_identifier,
            output_locked_assets_token_amount.token_nonce,
            &output_locked_assets_token_amount.amount,
            &self.asset_token_id().get(),
            &unlock_amount,
            &attributes,
            &output_locked_asset_attributes,
        );
        Ok(())
    }

    #[only_owner]
    #[endpoint(setUnlockPeriod)]
    fn set_unlock_period(
        &self,
        #[var_args] milestones: MultiArgVec<UnlockMilestone>,
    ) -> SCResult<()> {
        self.validate_unlock_milestones(&milestones)?;
        self.default_unlock_period()
            .set(&UnlockPeriod::from(milestones.into_vec()));
        Ok(())
    }

    fn get_month_start_epoch(&self, epoch: Epoch) -> Epoch {
        epoch - (epoch - self.init_epoch().get()) % EPOCHS_IN_MONTH
    }

    fn produce_tokens_and_send(
        &self,
        amount: &BigUint,
        attributes: &LockedAssetTokenAttributes,
        address: &ManagedAddress,
        opt_accept_funds_func: &OptionalArg<BoxedBytes>,
    ) -> SCResult<EsdtTokenPayment<Self::Api>> {
        let result = self.get_sft_nonce_for_unlock_schedule(&attributes.unlock_schedule);
        let sent_nonce = match result {
            Option::Some(cached_nonce) => {
                self.add_quantity_and_send_locked_assets(
                    amount,
                    cached_nonce,
                    address,
                    opt_accept_funds_func,
                )?;
                cached_nonce
            }
            Option::None => {
                let do_cache_result = !attributes.is_merged;

                let additional_amount_to_create = if do_cache_result {
                    self.types().big_uint_from(ADDITIONAL_AMOUNT_TO_CREATE)
                } else {
                    BigUint::zero()
                };

                let new_nonce = self.create_and_send_locked_assets(
                    amount,
                    &additional_amount_to_create,
                    address,
                    attributes,
                    opt_accept_funds_func,
                )?;

                if do_cache_result {
                    self.cache_unlock_schedule_and_nonce(&attributes.unlock_schedule, new_nonce);
                }
                new_nonce
            }
        };

        let token_id = self.locked_asset_token_id().get();
        Ok(self.create_payment(&token_id, sent_nonce, amount))
    }

    #[only_owner]
    #[payable("EGLD")]
    #[endpoint(issueLockedAssetToken)]
    fn issue_locked_asset_token(
        &self,
        token_display_name: ManagedBuffer,
        token_ticker: ManagedBuffer,
        #[payment_amount] issue_cost: BigUint,
    ) -> SCResult<AsyncCall> {
        require!(
            self.locked_asset_token_id().is_empty(),
            "NFT already issued"
        );

        Ok(self
            .send()
            .esdt_system_sc_proxy()
            .issue_semi_fungible(
                issue_cost,
                &token_display_name,
                &token_ticker,
                SemiFungibleTokenProperties {
                    can_add_special_roles: true,
                    can_change_owner: false,
                    can_freeze: false,
                    can_pause: false,
                    can_upgrade: true,
                    can_wipe: false,
                },
            )
            .async_call()
            .with_callback(self.callbacks().issue_nft_callback()))
    }

    #[callback]
    fn issue_nft_callback(&self, #[call_result] result: ManagedAsyncCallResult<(TokenIdentifier)>) {
        match result {
            ManagedAsyncCallResult::Ok(token_id) => {
                self.last_error_message().clear();

                if self.locked_asset_token_id().is_empty() {
                    self.locked_asset_token_id().set(&token_id);
                }
            }
            ManagedAsyncCallResult::Err(message) => {
                self.last_error_message().set(&message.err_msg);

                let (payment, token_id) = self.call_value().payment_token_pair();
                self.send().direct(
                    &self.blockchain().get_owner_address(),
                    &token_id,
                    0,
                    &payment,
                    &[],
                );
            }
        };
    }

    #[only_owner]
    #[endpoint(setLocalRolesLockedAssetToken)]
    fn set_local_roles_locked_asset_token(
        &self,
        address: ManagedAddress,
        #[var_args] roles: ManagedVarArgs<EsdtLocalRole>,
    ) -> SCResult<AsyncCall> {
        require!(
            !self.locked_asset_token_id().is_empty(),
            "Locked asset SFT not issued"
        );

        Ok(self
            .send()
            .esdt_system_sc_proxy()
            .set_special_roles(
                &address,
                &self.locked_asset_token_id().get(),
                roles.into_iter(),
            )
            .async_call()
            .with_callback(self.callbacks().change_roles_callback()))
    }

    #[callback]
    fn change_roles_callback(&self, #[call_result] result: ManagedAsyncCallResult<()>) {
        match result {
            ManagedAsyncCallResult::Ok(()) => {
                self.last_error_message().clear();
            }
            ManagedAsyncCallResult::Err(message) => {
                self.last_error_message().set(&message.err_msg);
            }
        }
    }

    fn create_unlock_schedule(
        &self,
        start_epoch: Epoch,
        unlock_period: UnlockPeriod,
    ) -> UnlockSchedule {
        UnlockSchedule {
            unlock_milestones: unlock_period
                .unlock_milestones
                .iter()
                .map(|x| UnlockMilestone {
                    unlock_epoch: x.unlock_epoch + start_epoch,
                    unlock_percent: x.unlock_percent,
                })
                .collect(),
        }
    }

    #[view(getLastErrorMessage)]
    #[storage_mapper("last_error_message")]
    fn last_error_message(&self) -> SingleValueMapper<ManagedBuffer>;

    #[view(getInitEpoch)]
    #[storage_mapper("init_epoch")]
    fn init_epoch(&self) -> SingleValueMapper<Epoch>;

    #[view(getWhitelistedContracts)]
    #[storage_mapper("whitelist")]
    fn whitelisted_contracts(&self) -> SetMapper<ManagedAddress>;

    #[view(getDefaultUnlockPeriod)]
    #[storage_mapper("default_unlock_period")]
    fn default_unlock_period(&self) -> SingleValueMapper<UnlockPeriod>;
}
