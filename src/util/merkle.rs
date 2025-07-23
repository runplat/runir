use std::collections::{BTreeSet, VecDeque};

use rayon::iter::{
    IndexedParallelIterator, IntoParallelRefIterator, ParallelBridge, ParallelIterator,
};
use sha2::{Digest, Sha256};

use crate::Data;

type Tree = Vec<Layer>;

type Layer = VecDeque<(BTreeSet<usize>, [u8; 32])>;

type Proof = Vec<Option<(bool, [u8; 32])>>;

pub fn build_merkle_tree(leaves: Vec<Data>) -> Tree {
    let mut levels = vec![];

    let mut current_level = leaves
        .iter()
        .map(|l| l.digest().finalize().into())
        .enumerate()
        .map(|(i, l)| (BTreeSet::from_iter(vec![i]), l))
        .inspect(|(i, _)| eprint!("{i:?} "))
        .collect::<VecDeque<(BTreeSet<usize>, [u8; 32])>>();

    eprintln!();
    levels.push(current_level.clone());

    while current_level.len() > 1 {
        let mut next_level = VecDeque::new();

        if current_level.len() == 2 {
            let (x, l) = &current_level[0];
            let (y, r) = &current_level[1];
            let mut xy = x.clone();
            xy.append(&mut y.clone());
            eprintln!("{xy:?}");
            let root = hash_concat(l, r);
            levels.push(VecDeque::from(vec![(xy, root)]));
            return levels;
        }
        let last: Option<(BTreeSet<usize>, [u8; 32])> = loop {
            match (current_level.pop_front(), current_level.pop_front()) {
                (Some((x, l)), Some((y, r))) => {
                    let mut xy = x.clone();
                    xy.append(&mut y.clone());
                    eprint!("{xy:?} ");
                    next_level.push_back((xy, hash_concat(&l, &r)));
                }
                (Some(l), None) => break Some(l),
                (None, None) => break None,
                _ => {
                    unreachable!()
                }
            }
        };

        if let Some(last) = last {
            eprint!("{:?}", last.0);
            next_level.push_back(last);
        }

        levels.push(next_level.clone());
        current_level = next_level;
        eprintln!()
    }

    levels
}

pub fn build_merkle_proof() {}

// pub fn build_merkle_proof(tree: &Tree, mut index: usize) -> Proof {
//     let mut proof = Vec::new();

//     // for (layer, level) in tree[..tree.len() - 1].iter().enumerate() {
//     //     // exclude root
//     //     let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };

//     //     if sibling_index < level.len() {
//     //         eprintln!("Sibling is {layer} / {sibling_index}");
//     //         proof.push(level[sibling_index]);
//     //     } else {
//     //         eprintln!("(odd {sibling_index}) Sibling is {layer} / {index}");
//     //         // odd-length level: duplicate last node
//     //         proof.push(level[index]);
//     //     }

//     //     index /= 2;
//     // }

//     for (layer, level) in tree[..tree.len() - 1].iter().enumerate() {
//         let is_left;
//         let sibling_index = if index % 2 == 0 {
//             // Right sibling
//             if index + 1 < level.len() {
//                 index + 1
//             } else {
//                 is_left = true;
//                 index - 1 // duplicate
//             }
//         } else {
//             is_left = true;
//             index - 1 // always exists
//         };

//         eprintln!("{layer} => {sibling_index}");
//         proof.push((is_left, level[sibling_index]));
//         index /= 2;
//     }

//     // for (layer, level) in tree[..tree.len() - 1].iter().enumerate() {
//     //     let sibling_index = match index % 2 {
//     //         0 => {
//     //             if index + 1 < level.len() {
//     //                 index + 1
//     //             } else {
//     //                 index - 1 // no sibling → duplicate
//     //             }
//     //         }
//     //         1 => index - 1,
//     //         _ => unreachable!(),
//     //     };

//     //     eprintln!("{layer} => {sibling_index}");
//     //     proof.push((sibling_index, level[sibling_index]));

//     //     // Here's the fix: instead of blindly dividing,
//     //     // you have to track which node this index *actually* became in the next level.
//     //     index /= 2;
//     // }

//     proof
// }

// #[inline]
// pub fn verify_merkle_proof(chunk: &[u8], proof: &Proof, root: &[u8; 32]) -> bool {
//     let mut hash: [u8; 32] = Sha256::digest(chunk).into();

//     for (is_left, sibling) in proof {
//         if *is_left {
//             // if sibling idx is even, sibling is on the left
//             hash = hash_concat(sibling, &hash);
//         } else {
//             hash = hash_concat(&hash, sibling);
//         }
//     }

//     &hash == root
// }

fn hash_concat(left: &[u8], right: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

#[cfg(test)]
mod test {
    use toml::toml;

    use crate::{Namespace, util::merkle::build_merkle_tree};

    #[test]
    fn test_build_merkle_tree() {
        let ns = Namespace::ephemeral();

        let leaves = vec![
            ns.store(
                "ex",
                &toml! {
                    name = "hello 1"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 2"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 3"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 4"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 5"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 6"
                },
            )
            .data,
            ns.store(
                "ex",
                &toml! {
                    name = "hello 7"
                },
            )
            .data,
        ];

        let tree = build_merkle_tree(leaves.clone());
        eprintln!(
            "tree: {:#?}",
            tree.iter().map(|l| l.len()).collect::<Vec<_>>()
        );

        // let proof = build_merkle_proof(&tree, 5);
        // eprintln!(
        //     "proof: {:#?}",
        //     proof
        //         .iter()
        //         .map(|l| (l.0, hex::encode(l.1)))
        //         .collect::<Vec<_>>()
        // );

        // assert!(verify_merkle_proof(
        //     &leaves[5],
        //     &proof,
        //     &tree.last().unwrap()[0]
        // ));
    }
}
