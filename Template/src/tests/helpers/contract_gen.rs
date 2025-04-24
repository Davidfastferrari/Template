use alloy::sol;

sol!(
    #[sol(rpc)]
    contract Router {
        struct ExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint24 fee;
            address recipient;
            uint256 amountIn;
            uint256 amountOutMinimum;
            uint160 sqrtPriceLimitX96;
        }
        function exactInputSingle(ExactInputSingleParams memory params)
        external
        returns (
            uint256 amountOut,
        );
    }
);

sol!(
    #[sol(rpc)]
    contract RouterDeadline {
        struct ExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint24 fee;
            address recipient;
            uint256 deadline;
            uint256 amountIn;
            uint256 amountOutMinimum;
            uint160 sqrtPriceLimitX96;
        }
        function exactInputSingle(ExactInputSingleParams memory params)
        external
        returns (
            uint256 amountOut,
        );
    }
);

sol!(
    #[sol(rpc)]
    contract ERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    contract BalancerV2Vault {
        enum SwapKind { GIVEN_IN, GIVEN_OUT }

        struct BatchSwapStep {
            bytes32 poolId;
            uint256 assetInIndex;
            uint256 assetOutIndex;
            uint256 amount;
            bytes userData;
        }

        struct FundManagement {
            address sender;
            bool fromInternalBalance;
            address payable recipient;
            bool toInternalBalance;
        }

        function queryBatchSwap(
            SwapKind kind,
            BatchSwapStep[] memory swaps,
            address[] memory assets,
            FundManagement memory funds
        ) external returns (int256[] memory);
    }
);

sol!(
    #[sol(rpc)]
    contract BalancerPool {
        function getPoolId() external view returns (bytes32);
    }
);

sol!(
    #[sol(rpc)]
    contract V3Quoter {
        struct QuoteExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint256 amountIn;
            uint24 fee;
            uint160 sqrtPriceLimitX96;
        }
        function quoteExactInputSingle(QuoteExactInputSingleParams memory params)
        external
        returns (
            uint256 amountOut,
            uint160 sqrtPriceX96After,
            uint32 initializedTicksCrossed,
            uint256 gasEstimate
        );

    }
);

sol!(
    #[sol(rpc)]
    contract V3QuoterSlipstream {
        struct QuoteExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint256 amountIn;
            int24 tickSpacing;
            uint160 sqrtPriceLimitX96;
        }
        function quoteExactInputSingle(QuoteExactInputSingleParams memory params)
        external
        returns (
            uint256 amountOut,
            uint160 sqrtPriceX96After,
            uint32 initializedTicksCrossed,
            uint256 gasEstimate
        );

    }
);

sol!(
    #[sol(rpc)]
    contract V2Router {
        function getAmountsOut(uint amountIn, address[] calldata path) external view returns (uint[] memory amounts);
    }
);

sol!(
    #[sol(rpc)]
    contract Aerodrome {
        function getAmountOut(uint256 amountIn, address tokenIn) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    contract Curve {
        function get_dy(uint256 i, uint256 j, uint256 dx) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    contract MaverickOut {
        function calculateSwap(
            address pool,
            uint128 amount,
            bool tokenAIn,
            bool exactOutput,
            int32 tickLimit
        ) external returns (uint256 amountIn, uint256 amountOut, uint256 gasEstimate);
    }
);

sol! {
    #[sol(rpc)]
    contract SlipstreamPool {
        function tickSpacing() external view returns (int24);
    }
}

