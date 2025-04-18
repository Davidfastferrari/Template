// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Interface definitions
interface IERC20 {
    function transfer(address, uint256) external returns (bool);
    function transferFrom(address, address, uint256) external returns (bool);
    function approve(address, uint256) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

interface IUniswapV2Pair {
    function getReserves() external view returns (uint112, uint112, uint32);
    function token0() external view returns (address);
    function token1() external view returns (address);
    function swap(uint, uint, address, bytes calldata) external;
    function factory() external view returns (address);
}

interface IUniswapV3Pool {
    function slot0() external view returns (uint160, int24, uint16, uint16, uint16, uint8, bool);
    function token0() external view returns (address);
    function token1() external view returns (address);
    function swap(address, bool, int256, uint160, bytes calldata) external returns (int256, int256);
}

contract FlashQuoter {
    struct SwapParams {
        address[] pools;
        uint8[] poolVersions;  // 0 for V2, 1 for V3
        uint256 amountIn;
    }

    // Constants to avoid multiple memory allocations
    bytes private constant EMPTY_BYTES = new bytes(0);
    uint256 private constant PRECISION = 10000;
    address private constant WETH = 0x4200000000000000000000000000000000000006;
    uint160 constant MIN_SQRT_RATIO = 4295128739;
    uint160 constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    // Top function that is called to quote an arbitarge path. The path is a valid path that is starting and ending in WETH
    function quoteArbitrage(SwapParams calldata params) external returns (uint256[] memory) {
        IERC20(WETH).transferFrom(msg.sender, address(this), params.amountIn);

        uint256[] memory amounts = new uint256[](params.pools.length + 1);
        amounts[0] = params.amountIn;
        
        // Track the input token for each swap
        address currentTokenIn = WETH;

        unchecked {
            for (uint256 i = 0; i < params.pools.length; i++) {
                address pool = params.pools[i];
                bool isV3 = params.poolVersions[i] == 1;
                
                address token0;
                address token1;
                if (isV3) {
                    IUniswapV3Pool v3Pool = IUniswapV3Pool(pool);
                    token0 = v3Pool.token0();
                    token1 = v3Pool.token1();
                } else {
                    IUniswapV2Pair v2Pool = IUniswapV2Pair(pool);
                    token0 = v2Pool.token0();
                    token1 = v2Pool.token1();
                }
                
                // Determine if we're going token0 -> token1
                bool zeroForOne = currentTokenIn == token0;
                
                // Approve and swap
                IERC20(currentTokenIn).approve(pool, amounts[i]);
                
                amounts[i + 1] = isV3 ? 
                    _swapV3(pool, amounts[i], currentTokenIn, zeroForOne) : 
                    _swapV2(pool, amounts[i], zeroForOne);
                
                // Set up the input token for the next swap
                currentTokenIn = zeroForOne ? token1 : token0;
            }
        }

        return amounts;
    }

    function _swapV2(
        address poolAddress, 
        uint256 amountIn,
        bool zeroForOne
    ) private returns (uint256 amountOut) {
        IUniswapV2Pair pair = IUniswapV2Pair(poolAddress);
        
        // Load reserves
        (uint112 reserve0, uint112 reserve1,) = pair.getReserves();
        
        // Get fee and transfer tokens
        uint16 fee = _getFee(pair.factory());
        address tokenIn = zeroForOne ? pair.token0() : pair.token1();
        IERC20(tokenIn).transfer(poolAddress, amountIn);
        
        // Calculate amount out using unchecked math where safe
        unchecked {
            uint256 reserveIn = uint256(zeroForOne ? reserve0 : reserve1);
            uint256 reserveOut = uint256(zeroForOne ? reserve1 : reserve0);
            
            uint256 amountInWithFee = amountIn * fee;
            amountOut = (amountInWithFee * reserveOut) / (reserveIn * PRECISION + amountInWithFee);
        }

        // Perform swap
        pair.swap(
            zeroForOne ? 0 : amountOut,
            zeroForOne ? amountOut : 0,
            address(this),
            EMPTY_BYTES
        );
    }

    function _swapV3(
        address poolAddress,
        uint256 amountIn,
        address tokenIn,
        bool zeroForOne
    ) private returns (uint256) {
        IUniswapV3Pool pool = IUniswapV3Pool(poolAddress);
        
        uint160 sqrtPriceLimitX96 = zeroForOne ? 
            MIN_SQRT_RATIO + 1 : 
            MAX_SQRT_RATIO - 1;

        (int256 amount0, int256 amount1) = pool.swap(
            address(this),             // recipient
            zeroForOne,               // direction
            int256(amountIn),         // amount
            sqrtPriceLimitX96,        // price limit
            abi.encode(               // callback data
                poolAddress,          // to
                tokenIn              // tokenIn
            )
        );

        return uint256(-(zeroForOne ? amount1 : amount0));
    }

    function uniswapV3SwapCallback(
        int256 amount0Delta,
        int256 amount1Delta,
        bytes calldata data
    ) external {
        (address to, address tokenIn) = abi.decode(data, (address, address));
        uint256 amountToSend = uint256(amount0Delta > 0 ? amount0Delta : amount1Delta);
        IERC20(tokenIn).transfer(to, amountToSend);
    }


    // Get the fee for the factory
    function _getFee(address factory) private pure returns (uint16) {
        if (factory == 0x8909Dc15e40173Ff4699343b6eB8132c65e18eC6) return 9970;
        if (factory == 0x71524B4f93c58fcbF659783284E38825f0622859) return 9970;
        if (factory == 0x02a84c1b3BBD7401a5f7fa98a384EBC70bB5749E) return 9975;
        if (factory == 0x04C9f118d21e8B767D2e50C946f0cC9F6C367300) return 9970;
        if (factory == 0xFDa619b6d20975be80A10332cD39b9a4b0FAa8BB) return 9975;
        if (factory == 0x591f122D1df761E616c13d265006fcbf4c6d6551) return 9975;
        if (factory == 0x3E84D913803b02A4a7f027165E8cA42C14C0FdE7) return 9984;
        return 9970; // Default fee
    }

    receive() external payable {}
}
