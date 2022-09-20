#![no_std]

elrond_wasm::imports!();
elrond_wasm::derive_imports!();

#[elrond_wasm::module]
pub trait TokenSendModule {
    fn send_multiple_tokens(
        &self,
        destination: &ManagedAddress,
        payments: &[EsdtTokenPayment<Self::Api>],
        opt_accept_funds_func: &OptionalArg<BoxedBytes>,
    ) -> SCResult<()> {
        let (function, gas_limit) = match opt_accept_funds_func {
            OptionalArg::Some(accept_funds_func) => (
                accept_funds_func.as_slice(),
                self.transfer_exec_gas_limit().get(),
            ),
            OptionalArg::None => {
                let no_func: &[u8] = &[];
                (no_func, 0u64)
            }
        };

        self.raw_vm_api()
            .direct_multi_esdt_transfer_execute(
                destination,
                &ManagedVec::managed_from(self.type_manager(), payments.to_vec()),
                gas_limit,
                &ManagedBuffer::managed_from(self.type_manager(), function),
                &ManagedArgBuffer::new_empty(self.type_manager()),
            )
            .into()
    }

    fn send_multiple_tokens_compact(
        &self,
        destination: &ManagedAddress,
        payments: &[EsdtTokenPayment<Self::Api>],
        opt_accept_funds_func: &OptionalArg<BoxedBytes>,
    ) -> SCResult<()> {
        let mut compact_payments = Vec::<EsdtTokenPayment<Self::Api>>::new();
        for payment in payments.iter() {
            if payment.amount != 0 {
                let existing_index = compact_payments.iter().position(|x| {
                    x.token_identifier == payment.token_identifier
                        && x.token_nonce == payment.token_nonce
                });

                match existing_index {
                    Some(index) => compact_payments[index].amount += &payment.amount,
                    None => compact_payments.push(payment.clone()),
                }
            }
        }

        match compact_payments.len() {
            0 => Ok(()),
            _ => self.send_multiple_tokens(destination, &compact_payments, opt_accept_funds_func),
        }
    }

    fn transfer_execute_custom(
        &self,
        to: &ManagedAddress,
        token: &TokenIdentifier,
        nonce: u64,
        amount: &BigUint,
        opt_accept_funds_func: &OptionalArg<BoxedBytes>,
    ) -> SCResult<()> {
        if amount == &0 {
            return Ok(());
        }

        let (function, gas_limit) = match opt_accept_funds_func {
            OptionalArg::Some(accept_funds_func) => (
                accept_funds_func.as_slice(),
                self.transfer_exec_gas_limit().get(),
            ),
            OptionalArg::None => {
                let no_func: &[u8] = &[];
                (no_func, 0u64)
            }
        };
        let arg_buffer = ManagedArgBuffer::new_empty(self.type_manager());
        let endpoint_name = ManagedBuffer::new_from_bytes(self.type_manager(), function);
        let mut payments = ManagedVec::new();
        payments.push(EsdtTokenPayment::from(token.clone(), nonce, amount.clone()));

        self.raw_vm_api()
            .direct_multi_esdt_transfer_execute(
                to,
                &payments,
                gas_limit,
                &endpoint_name,
                &arg_buffer,
            )
            .into()
    }

    fn get_all_payments(&self) -> Vec<EsdtTokenPayment<Self::Api>> {
        self.raw_vm_api()
            .get_all_esdt_transfers()
            .into_iter()
            .collect()
    }

    fn create_payment(
        &self,
        token_id: &TokenIdentifier,
        nonce: u64,
        amount: &BigUint,
    ) -> EsdtTokenPayment<Self::Api> {
        EsdtTokenPayment::from(token_id.clone(), nonce, amount.clone())
    }

    #[view(getTransferExecGasLimit)]
    #[storage_mapper("transfer_exec_gas_limit")]
    fn transfer_exec_gas_limit(&self) -> SingleValueMapper<u64>;
}
