use 0x2812a8151dca68f0e49f5e6eb5ecab33::MyToken;
use 0x0::LibraAccount;

fun main(payee: address, auth_key_prefix: vector<u8>, amount: u64) {
    LibraAccount::pay_from_sender<MyToken::T>(payee, auth_key_prefix, amount);
}