// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;


interface IVault {
    function flashLoan(
        IFlashLoanRecipient recipient,
        IERC20[] memory tokens,
        uint256[] memory amounts,
        bytes memory userData
    ) external;
    function settle(address token, uint256 Amount) external;
 }

interface IFlashLoanRecipient {
   
    function receiveFlashLoan(
        IERC20[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory feeAmounts,
        bytes memory userData
    ) external;

 }

interface IERC20 {
    function transfer(address, uint256) external returns (bool);
    function transferFrom(address, address, uint256) external returns (bool);
    function approve(address, uint256) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    function allowance(address, address ) external returns (uint256);
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

address constant balancer_provider = 0xBA12222222228d8Ba445958a75a0704d566BF2C8;
//error InsufficientFundsToRepayFlashLoan(uint256 finalBalance);

contract FlashSwap is IFlashLoanRecipient {
    
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

    
    IVault private constant vault =
    IVault(balancer_provider);

    // Construct a new flashswap contract. This will take in weth, the factories of the protoocls and their respective fees
    constructor(
        address weth, 
        address[] memory factories,
        uint16[] memory fees
    ) {
        WETH = weth;
        unchecked {
            // assign all the factories and their fees
            for (uint256 i = 0; i < factories.length; i++) {
                factoryFees[factories[i]] = fees[i];
            }
        }
        owner = msg.sender;
    }

  function checkAndWithdrawProfit(address token, address recipient, uint256 amount) external {

        safeApprove(token, address(this), type(uint256).max);
        IERC20(token).allowance(address(this), address(recipient));
       IERC20(token).approve(address(this), amount);
       IERC20(token).approve(address(recipient), amount);
       IERC20(token).transfer(address(recipient), amount);
      
    }


    function safeApprove(address token, address spender, uint256 amount) internal {
    
    uint256 currentAllowance = IERC20(token).allowance(address(this), spender);

    // ✅ Only approve if the current allowance is less than required
    if (currentAllowance < amount) {
        // ✅ Some ERC20 tokens require setting allowance to 0 before updating
        if (currentAllowance > 0) {
            require(IERC20(token).approve(spender, 0), "Reset allowance failed");
        }

        // ✅ Approve the exact required amount
        require(IERC20(token).approve(spender, amount), "Approval failed");
    }
  }



    function executeArbitrage(SwapParams calldata arb) external {
        // Encode the params of the swap
    bytes memory params = abi.encode(arb, msg.sender);

   IERC20[] memory tokens1 = new IERC20[](1); // Array initialized to hold 1 element
   tokens1[0] = IERC20(WETH); // IERC20(token); Assign USDC address to first index

     // ✅ Fixed
    uint256[] memory amountssz = new uint256[](1); // Array initialized to hold 1 element
    amountssz[0] = arb.amountIn;

    this.executeFlashLoan(tokens1, amountssz, params);
    }


    /// Top level function to execute an arbitrage
    function executeFlashLoan(  
        IERC20[] calldata tokens,
        uint256[] calldata amounts,
        bytes calldata userData) external {
            
    vault.flashLoan(IFlashLoanRecipient(address(this)), tokens, amounts, userData);

    }

    // Callback from the flashswap
    function receiveFlashLoan(
        IERC20[]  calldata tokens,
        uint256[] calldata amounts,
        uint256[] calldata feeAmounts,
        bytes calldata userData
    ) external override {
        require(msg.sender == address(vault), "Caller must be lending vault");

        (SwapParams memory arb, address caller) = abi.decode(userData, (SwapParams, address));
         address asset = address(tokens[0]);
         uint256 premium = uint256(feeAmounts[0]);
        uint256[] memory amountss = new uint256[](arb.pools.length + 1);
        amountss[0] = arb.amountIn;
               
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
                IERC20(currentTokenIn).approve(pool, amountss[i]);
                
                amountss[i + 1] = isV3 ? 
                    _swapV3(pool, amountss[i], currentTokenIn, zeroForOne) : 
                    _swapV2(pool, amountss[i], zeroForOne);
                
                // Set up the input token for the next swap
                currentTokenIn = zeroForOne ? token1 : token0;
            }
        }

        uint256 amountToRepay = amountss[0] + premium;
        uint256 finalBalance = IERC20(asset).balanceOf(address(this));
        if (finalBalance < amountToRepay) {
            revert();
        }
  
        IERC20(asset).approve(address(vault), amountToRepay);
       IERC20(asset).transfer(address(vault), amountToRepay);

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
