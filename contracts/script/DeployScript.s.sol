// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Script.sol";
import "../src/FlashSwap.sol";

contract DeployFlashSwap is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerPrivateKey);

        // Set up factory addresses
        address[] memory factories = new address[](7);
        factories[0] = 0x8909Dc15e40173Ff4699343b6eB8132c65e18eC6;  // 0.3%
        factories[1] = 0x71524B4f93c58fcbF659783284E38825f0622859;  // 0.3%
        factories[2] = 0x02a84c1b3BBD7401a5f7fa98a384EBC70bB5749E;  // 0.25%
        factories[3] = 0x04C9f118d21e8B767D2e50C946f0cC9F6C367300;  // 0.3%
        factories[4] = 0xFDa619b6d20975be80A10332cD39b9a4b0FAa8BB;  // 0.25%
        factories[5] = 0x591f122D1df761E616c13d265006fcbf4c6d6551;  // 0.25%
        factories[6] = 0x3E84D913803b02A4a7f027165E8cA42C14C0FdE7;  // 0.16%

        // Set up corresponding fees (10000 = 100%)
        uint16[] memory fees = new uint16[](7);
        fees[0] = 9970; // 0.3%
        fees[1] = 9970; // 0.3%
        fees[2] = 9975; // 0.25%
        fees[3] = 9970; // 0.3%
        fees[4] = 9975; // 0.25%
        fees[5] = 9975; // 0.25%
        fees[6] = 9984; // 0.16%

        // Deploy with WETH address for Base
        address WETH = 0x4200000000000000000000000000000000000006;
        
        FlashSwap flashSwap = new FlashSwap(
            WETH,
            factories,
            fees
        );

        console.log("FlashSwap deployed at:", address(flashSwap));

        vm.stopBroadcast();
    }
}
