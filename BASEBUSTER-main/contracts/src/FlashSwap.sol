// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "@aave/core-v3/contracts/flashloan/base/FlashLoanSimpleReceiverBase.sol";

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

address constant AAVE_ADDRESS_PROVIDER = 0xe20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D;
error InsufficientFundsToRepayFlashLoan(uint256 finalBalance);

contract FlashSwap is FlashLoanSimpleReceiverBase {

    struct SwapParams {
        address[] pools;        // Array of pool addresses in swap order
        uint8[] poolVersions;   // 0 = V2, 1 = V3
        uint256 amountIn;
    }

    // Mapping from a factory to its fee
    mapping(address => uint16) private factoryFees;
    address private immutable WETH;
    address public owner;

    // Constants to avoid multiple memory allocations
    bytes private constant EMPTY_BYTES = new bytes(0);
    uint256 private constant PRECISION = 10000;
    uint160 constant MIN_SQRT_RATIO = 4295128739;
    uint160 constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    // Construct a new flashswap contract. This will take in weth, the factories of the protoocls and their respective fees
    constructor(
        address weth, 
        address[] memory factories,
        uint16[] memory fees
    ) FlashLoanSimpleReceiverBase(IPoolAddressesProvider(AAVE_ADDRESS_PROVIDER)) {
        WETH = weth;
        unchecked {
            // assign all the factories and their fees
            for (uint256 i = 0; i < factories.length; i++) {
                factoryFees[factories[i]] = fees[i];
            }
        }
        owner = msg.sender;
    }


    /// Top level function to execute an arbitrage
    function executeArbitrage(SwapParams calldata arb) external {
        // Encode the params of the swap
        bytes memory params = abi.encode(arb, msg.sender);
        POOL.flashLoanSimple(address(this), WETH, arb.amountIn, params, 0);
    }

    // Callback from the flashswap
    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address,
        bytes calldata params
    ) external returns (bool) {
        require(msg.sender == address(POOL), "Caller must be lending pool");

        (SwapParams memory arb, address caller) = abi.decode(params, (SwapParams, address));

        uint256[] memory amounts = new uint256[](arb.pools.length + 1);
        amounts[0] = arb.amountIn;

         // Track the input token for each swap
        address currentTokenIn = WETH;

        unchecked {
            for (uint256 i = 0; i < arb.pools.length; i++) {
                address pool = arb.pools[i];
                bool isV3 = arb.poolVersions[i] == 1;
                
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

        uint256 amountToRepay = amount + premium;
        uint256 finalBalance = IERC20(asset).balanceOf(address(this));
        if (finalBalance < amountToRepay) {
            revert();
        }

        IERC20(asset).approve(address(POOL), amountToRepay);
        IERC20(asset).transfer(caller, finalBalance - amountToRepay);

        return true;
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
        uint16 fee = factoryFees[pair.factory()];
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

    receive() external payable {}
}
