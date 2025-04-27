use alloy::primitives::Address;
use alloy::sol;
use petgraph::graph::UnGraph;
use petgraph::prelude::*;
use pool_sync::{BalancerV2Pool, CurveTriCryptoPool, Pool};
use std::collections::HashSet;
use std::hash::{Hash, Hasher, DefaultHasher};

use crate::swap::{SwapPath, SwapStep};

pub struct ArbGraph;

impl ArbGraph {
    /// Generate arbitrage cycles using known pools
    pub async fn generate_cycles(working_pools: Vec<Pool>) -> Vec<SwapPath> {
        // Fetch token (e.g. WETH) as starting point from env
        let token: Address = std::env::var("WETH")
            .expect("WETH environment variable must be set")
            .parse()
            .expect("Invalid WETH address");

        let graph = Self::build_graph(working_pools).await;

        let start_node = graph
            .node_indices()
            .find(|node| graph[*node] == token)
            .expect("Start token not found in graph");

        let cycles = Self::find_all_arbitrage_paths(&graph, start_node, 2);

        // Hash & structure the cycles
        cycles
            .into_iter()
            .map(|cycle| {
                let mut hasher = DefaultHasher::new();
                for step in &cycle {
                    step.hash(&mut hasher);
                }

                SwapPath {
                    steps: cycle,
                    hash: hasher.finish(),
                }
            })
            .collect()
    }

    /// Build token connectivity graph from pool list
    async fn build_graph(working_pools: Vec<Pool>) -> UnGraph<Address, Pool> {
        let mut graph: UnGraph<Address, Pool> = UnGraph::new_undirected();
        let mut inserted_nodes: HashSet<Address> = HashSet::new();

        for pool in working_pools {
            match pool {
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
                    Self::add_simple_pool_to_graph(&mut graph, &mut inserted_nodes, pool);
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

        for token in [token0, token1] {
            if inserted_nodes.insert(token) {
                graph.add_node(token);
            }
        }

        let node0 = graph
            .node_indices()
            .find(|&n| graph[n] == token0)
            .expect("Token0 not found in graph");
        let node1 = graph
            .node_indices()
            .find(|&n| graph[n] == token1)
            .expect("Token1 not found in graph");

        graph.add_edge(node0, node1, pool);
    }

    fn add_curve_pool_to_graph(
        graph: &mut UnGraph<Address, Pool>,
        inserted_nodes: &mut HashSet<Address>,
        curve_pool: CurveTriCryptoPool,
    ) {
        let tokens = curve_pool.get_tokens();

        for &token in &tokens {
            if inserted_nodes.insert(token) {
                graph.add_node(token);
            }
        }

        for (i, &token_in) in tokens.iter().enumerate() {
            for &token_out in tokens.iter().skip(i + 1) {
                let node_in = graph
                    .node_indices()
                    .find(|&n| graph[n] == token_in)
                    .expect("Token_in not found");
                let node_out = graph
                    .node_indices()
                    .find(|&n| graph[n] == token_out)
                    .expect("Token_out not found");

                graph.add_edge(node_in, node_out, Pool::CurveTriCrypto(curve_pool.clone()));
            }
        }
    }

    fn add_balancer_pool_to_graph(
        graph: &mut UnGraph<Address, Pool>,
        inserted_nodes: &mut HashSet<Address>,
        balancer_pool: BalancerV2Pool,
    ) {
        let tokens = balancer_pool.get_tokens();

        for &token in &tokens {
            if inserted_nodes.insert(token) {
                graph.add_node(token);
            }
        }

        for (i, &token_in) in tokens.iter().enumerate() {
            for &token_out in tokens.iter().skip(i + 1) {
                let balance_in = balancer_pool.get_balance(&token_in);
                let balance_out = balancer_pool.get_balance(&token_out);

                if !balance_in.is_zero() && !balance_out.is_zero() {
                    let node_in = graph
                        .node_indices()
                        .find(|&n| graph[n] == token_in)
                        .expect("Token_in not found");
                    let node_out = graph
                        .node_indices()
                        .find(|&n| graph[n] == token_out)
                        .expect("Token_out not found");

                    graph.add_edge(node_in, node_out, Pool::BalancerV2(balancer_pool.clone()));
                }
            }
        }
    }

    /// Finds arbitrage paths starting and ending at the same node
    fn find_all_arbitrage_paths(
        graph: &UnGraph<Address, Pool>,
        start_node: NodeIndex,
        max_hops: usize,
    ) -> Vec<Vec<SwapStep>> {
        let mut all_paths = Vec::new();
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

    /// Recursively builds cycles from token paths
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

                    let swap_path = new_path
                        .iter()
                        .map(|(base, pool, quote)| SwapStep {
                            pool_address: pool.address(),
                            token_in: graph[*base],
                            token_out: graph[*quote],
                            protocol: pool.pool_type(),
                            fee: pool.fee(),
                        })
                        .collect();

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
