// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Script.sol";
import "../src/FlashQuoter.sol";

contract DeployFlashQuoter is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerPrivateKey);

        FlashQuoter flashQuoter = new FlashQuoter();

        console.log("FlashSwap deployed at:", address(flashQuoter));

        vm.stopBroadcast();
    }
}