use 0x2812a8151dca68f0e49f5e6eb5ecab33::MyToken;
use 0x0::LibraAccount;

fun main() {
    // Create 'Balance<Token>' resource under sender account to receive token
    LibraAccount::create_new_balance<MyToken::T>();
}