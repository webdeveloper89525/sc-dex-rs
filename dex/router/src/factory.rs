elrond_wasm::imports!();
elrond_wasm::derive_imports!();

use core::iter::FromIterator;
const TEMPORARY_OWNER_PERIOD_BLOCKS: u64 = 50;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, PartialEq, TypeAbi)]
pub struct PairTokens<M: ManagedTypeApi> {
    pub first_token_id: TokenIdentifier<M>,
    pub second_token_id: TokenIdentifier<M>,
}

#[derive(TopEncode, TopDecode, PartialEq, TypeAbi)]
pub struct PairContractMetadata<M: ManagedTypeApi> {
    first_token_id: TokenIdentifier<M>,
    second_token_id: TokenIdentifier<M>,
    address: ManagedAddress<M>,
}

#[elrond_wasm::module]
pub trait FactoryModule {
    fn init_factory(&self, pair_template_address_opt: Option<ManagedAddress>) {
        if let Some(addr) = pair_template_address_opt {
            self.pair_template_address().set(&addr);
        }

        self.temporary_owner_period()
            .set_if_empty(&TEMPORARY_OWNER_PERIOD_BLOCKS);
    }

    fn create_pair(
        &self,
        first_token_id: &TokenIdentifier,
        second_token_id: &TokenIdentifier,
        owner: &ManagedAddress,
        total_fee_percent: u64,
        special_fee_percent: u64,
    ) -> SCResult<ManagedAddress> {
        require!(
            !self.pair_template_address().is_empty(),
            "pair contract template is empty"
        );

        let mut arg_buffer = ManagedArgBuffer::new_empty(self.type_manager());
        arg_buffer.push_arg(first_token_id);
        arg_buffer.push_arg(second_token_id);
        arg_buffer.push_arg(self.blockchain().get_sc_address());
        arg_buffer.push_arg(owner);
        arg_buffer.push_arg(&total_fee_percent.to_be_bytes()[..]);
        arg_buffer.push_arg(&special_fee_percent.to_be_bytes()[..]);

        let (new_address, _) = self.raw_vm_api().deploy_from_source_contract(
            self.blockchain().get_gas_left(),
            &BigUint::zero(),
            &self.pair_template_address().get(),
            CodeMetadata::UPGRADEABLE,
            &arg_buffer,
        );

        self.pair_map().insert(
            PairTokens {
                first_token_id: first_token_id.clone(),
                second_token_id: second_token_id.clone(),
            },
            new_address.clone(),
        );
        self.pair_temporary_owner().insert(
            new_address.clone(),
            (
                self.blockchain().get_caller(),
                self.blockchain().get_block_nonce(),
            ),
        );
        Ok(new_address)
    }

    fn upgrade_pair(
        &self,
        pair_address: &ManagedAddress,
        first_token_id: &TokenIdentifier,
        second_token_id: &TokenIdentifier,
        owner: &ManagedAddress,
        total_fee_percent: u64,
        special_fee_percent: u64,
    ) {
        let mut arg_buffer = ManagedArgBuffer::new_empty(self.type_manager());
        arg_buffer.push_arg(first_token_id);
        arg_buffer.push_arg(second_token_id);
        arg_buffer.push_arg(self.blockchain().get_sc_address());
        arg_buffer.push_arg(owner);
        arg_buffer.push_arg(&total_fee_percent.to_be_bytes()[..]);
        arg_buffer.push_arg(&special_fee_percent.to_be_bytes()[..]);

        self.raw_vm_api().upgrade_from_source_contract(
            pair_address,
            self.blockchain().get_gas_left(),
            &BigUint::zero(),
            &self.pair_template_address().get(),
            CodeMetadata::UPGRADEABLE,
            &arg_buffer,
        );
    }

    #[storage_mapper("pair_map")]
    fn pair_map(&self) -> MapMapper<PairTokens<Self::Api>, ManagedAddress>;

    #[view(getAllPairsManagedAddresses)]
    fn get_all_pairs_addresses(&self) -> MultiResultVec<ManagedAddress> {
        self.pair_map().values().collect()
    }

    #[view(getAllPairTokens)]
    fn get_all_token_pairs(&self) -> MultiResultVec<PairTokens<Self::Api>> {
        self.pair_map().keys().collect()
    }

    #[view(getAllPairContractMetadata)]
    fn get_all_pair_contract_metadata(&self) -> MultiResultVec<PairContractMetadata<Self::Api>> {
        let map: Vec<PairContractMetadata<Self::Api>> = self
            .pair_map()
            .iter()
            .map(|x| PairContractMetadata {
                first_token_id: x.0.first_token_id,
                second_token_id: x.0.second_token_id,
                address: x.1,
            })
            .collect();
        MultiResultVec::from_iter(map)
    }

    fn get_pair_temporary_owner(&self, pair_address: &ManagedAddress) -> Option<ManagedAddress> {
        let result = self.pair_temporary_owner().get(pair_address);

        match result {
            Some((temporary_owner, creation_block)) => {
                let expire_block = creation_block + self.temporary_owner_period().get();

                if expire_block <= self.blockchain().get_block_nonce() {
                    self.pair_temporary_owner().remove(pair_address);
                    None
                } else {
                    Some(temporary_owner)
                }
            }
            None => None,
        }
    }

    #[only_owner]
    #[endpoint(clearPairTemporaryOwnerStorage)]
    fn clear_pair_temporary_owner_storage(&self) -> usize {
        let size = self.pair_temporary_owner().len();
        self.pair_temporary_owner().clear();
        size
    }

    #[only_owner]
    #[endpoint(setTemporaryOwnerPeriod)]
    fn set_temporary_owner_period(&self, period_blocks: u64) {
        self.temporary_owner_period().set(&period_blocks);
    }

    #[only_owner]
    #[endpoint(setPairTemplateAddress)]
    fn set_pair_template_address(&self, address: ManagedAddress) {
        self.pair_template_address().set(&address);
    }

    #[view(getPairTemplateAddress)]
    #[storage_mapper("pair_template_address")]
    fn pair_template_address(&self) -> SingleValueMapper<ManagedAddress>;

    #[view(getTemporaryOwnerPeriod)]
    #[storage_mapper("temporary_owner_period")]
    fn temporary_owner_period(&self) -> SingleValueMapper<u64>;

    #[storage_mapper("pair_temporary_owner")]
    fn pair_temporary_owner(&self) -> MapMapper<ManagedAddress, (ManagedAddress, u64)>;
}
