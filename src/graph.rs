use crate::swap::{SwapPath, SwapStep};
use alloy::alloy-primitives::Address;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::prelude;
use pool_sync::{UniswapV2, UniswapV3, SushiswapV2, SushiswapV3, PancakeswapV2, PancakeswapV3, BalancerV2Pool, BaseswapV2, BaseswapV3, MaverickV1, MaverickV2, Aerodrome, Slipstream, AlienBase, CurveTriCryptoPool, Pool, PoolSync, PoolType, Chain, PoolInfo};
use std::collections::HashSet;
use std::hash::Hash;
use std::hash::{DefaultHasher, Hasher};

pub struct ArbGraph;
impl ArbGraph {
    // Constructor, takes the set of working tokens we are interested in searching over
    pub async fn generate_cycles(working_pools: Vec<Pool>) -> Vec<SwapPath> {
        // build the graph
        let token: Address = std::env::var("WETH").unwrap().parse().unwrap();
        let graph = ArbGraph::build_graph(working_pools);

        // get start node and construct cycles
        let start_node = graph
            .node_indices()
            .find(|node| graph[*node] == token)
            .unwrap();
        let cycles = ArbGraph::find_all_arbitrage_paths(&graph, start_node, 2);

        // form our swappaths
        let swappaths: Vec<SwapPath> = cycles
            .iter()
            .map(|cycle| {
                let mut hasher = DefaultHasher::new();
                cycle.iter().for_each(|step| step.hash(&mut hasher));
                let output_hash = hasher.finish();
                SwapPath {
                    steps: cycle.clone(),
                    hash: output_hash,
                }
            })
            .collect();
        swappaths
    }

    // Build the graph from the working set of pools
   async fn build_graph(working_pools: Vec<Pool>) -> UnGraph<Address, Pool> {
        let mut graph: UnGraph<Address, Pool> = UnGraph::new_undirected();
        let mut inserted_nodes: HashSet<Address> = HashSet::new();
        let pool_sync = PoolSync::builder()
        .add_pool(PoolType::UniswapV2, UniswapV3, SushiswapV2, SushiswapV3, PancakeswapV2, PancakeswapV3, BaseswapV2, BaseswapV3, MaverickV1, MaverickV2, Aerodrome, Slipstream, AlienBase)
        .chain(Chain::Base)
        .build()?;
        
      // Synchronize pools
       let (pools, last_synced_block) = pool_sync.sync_pools().await?;

        for pools in working_pools {
            // add the nodes ot the graph if they have not already been added
            match pools {
               Pool::BalancerV2(balancer_pool) => {
                    Self::add_balancer_pool_to_graph(
                        &mut graph,
                        &mut inserted_nodes,
                        balancer_pool,
                    );
                }
                Pool::CurveTriCrypto(curve_pool) => {
                    Self::add_curve_pool_to_graph(&mut graph, &mut inserted_nodes, curve_pool);
                }
                _ => {
                    Self::add_simple_pool_to_graph(&mut graph, &mut inserted_nodes, pools);
                }
            }
        }
        graph
    }

    fn add_simple_pool_to_graph(
        graph: &mut UnGraph<Address, Pool>,
        inserted_nodes: &mut HashSet<Address>,
        pool: Pool,
    ) {
        let token0 = pool.token0_address();
        let token1 = pool.token1_address();

        // Add nodes if they don't exist
        if !inserted_nodes.contains(&token0) {
            graph.add_node(token0);
            inserted_nodes.insert(token0);
        }
        if !inserted_nodes.contains(&token1) {
            graph.add_node(token1);
            inserted_nodes.insert(token1);
        }

        // Get the node indices
        let node0 = graph.node_indices().find(|&n| graph[n] == token0).unwrap();
        let node1 = graph.node_indices().find(|&n| graph[n] == token1).unwrap();

        // Add the edge (pool)
        graph.add_edge(node0, node1, pool);
    }

    fn add_curve_pool_to_graph(
        graph: &mut UnGraph<Address, Pool>,
        inserted_nodes: &mut HashSet<Address>,
        curve_pool: CurveTriCryptoPool,
    ) {
        let tokens = curve_pool.get_tokens();

        // Add nodes for all tokens in the pool
        for &token in &tokens {
            if !inserted_nodes.contains(&token) {
                graph.add_node(token);
                inserted_nodes.insert(token);
            }
        }

        // Add edges for all possible token pairs with non-zero balances
        for (i, &token_in) in tokens.iter().enumerate() {
            for &token_out in tokens.iter().skip(i + 1) {
                // Only add edge if both balances are non-zero
                let node_in = graph
                    .node_indices()
                    .find(|&n| graph[n] == token_in)
                    .unwrap();
                let node_out = graph
                    .node_indices()
                    .find(|&n| graph[n] == token_out)
                    .unwrap();

                // Create a new Pool::BalancerV2 for each edge
                let pool = Pool::CurveTriCrypto(curve_pool.clone());
                graph.add_edge(node_in, node_out, pool);
            }
        }
    }

    fn add_balancer_pool_to_graph(
        graph: &mut UnGraph<Address, Pool>,
        inserted_nodes: &mut HashSet<Address>,
        balancer_pool: BalancerV2Pool,
    ) {
        let tokens = balancer_pool.get_tokens();

        // Add nodes for all tokens in the pool
        for &token in &tokens {
            if !inserted_nodes.contains(&token) {
                graph.add_node(token);
                inserted_nodes.insert(token);
            }
        }

        // Add edges for all possible token pairs with non-zero balances
        for (i, &token_in) in tokens.iter().enumerate() {
            for &token_out in tokens.iter().skip(i + 1) {
                let balance_in = balancer_pool.get_balance(&token_in);
                let balance_out = balancer_pool.get_balance(&token_out);

                // Only add edge if both balances are non-zero
                if !balance_in.is_zero() && !balance_out.is_zero() {
                    let node_in = graph
                        .node_indices()
                        .find(|&n| graph[n] == token_in)
                        .unwrap();
                    let node_out = graph
                        .node_indices()
                        .find(|&n| graph[n] == token_out)
                        .unwrap();

                    // Create a new Pool::BalancerV2 for each edge
                    let pool = Pool::BalancerV2(balancer_pool.clone());
                    graph.add_edge(node_in, node_out, pool);
                }
            }
        }
    }

    fn find_all_arbitrage_paths(
        graph: &UnGraph<Address, Pool>,
        start_node: NodeIndex,
        max_hops: usize,
    ) -> Vec<Vec<SwapStep>> {
        //let mut all_paths = Vec::new();
        let mut all_paths: Vec<Vec<SwapStep>> = Vec::new();
        let mut current_path = Vec::new();
        let mut visited = HashSet::new();

        Self::construct_cycles(
            graph,
            start_node,
            start_node,
            max_hops,
            &mut current_path,
            &mut visited,
            &mut all_paths,
        );

        all_paths
    }

    // Build all of the cycles
    fn construct_cycles(
        graph: &UnGraph<Address, Pool>,
        current_node: NodeIndex,
        start_node: NodeIndex,
        max_hops: usize,
        current_path: &mut Vec<(NodeIndex, Pool, NodeIndex)>,
        visited: &mut HashSet<NodeIndex>,
        all_paths: &mut Vec<Vec<SwapStep>>,
    ) {
        if current_path.len() >= max_hops {
            return;
        }

        for edge in graph.edges(current_node) {
            let next_node = edge.target();
            let protocol = edge.weight().clone();

            if next_node == start_node {
                if current_path.len() >= 2
                    || (current_path.len() == 1
                        && current_path[0].1.pool_type() != protocol.pool_type())
                {
                    let mut new_path = current_path.clone();
                    new_path.push((current_node, protocol, next_node));

                    let mut swap_path = Vec::new();
                    for (base, pool, quote) in new_path.iter() {
                        let swap = SwapStep {
                            pool_address: pool.address(),
                            token_in: graph[*base],
                            token_out: graph[*quote],
                            protocol: pool.pool_type(),
                            fee: pool.fee(),
                        };
                        swap_path.push(swap);
                    }

                    all_paths.push(swap_path);
                }
            } else if !visited.contains(&next_node) {
                current_path.push((current_node, protocol, next_node));
                visited.insert(next_node);

                Self::construct_cycles(
                    graph,
                    next_node,
                    start_node,
                    max_hops,
                    current_path,
                    visited,
                    all_paths,
                );

                current_path.pop();
                visited.remove(&next_node);
            }
        }
    }
}
