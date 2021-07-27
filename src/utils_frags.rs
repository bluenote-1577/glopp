use crate::types_structs::Frag;
use crate::types_structs::HapBlock;
use fxhash::{FxHashMap, FxHashSet};

// Get the number # of different bases between the
// two fragments
pub fn distance(r1: &Frag, r2: &Frag) -> (i32,i32) {
    let mut diff = 0;
    let mut same = 0;

    for pos in r1.positions.intersection(&r2.positions) {
        if r1.seq_dict.get(pos) == r2.seq_dict.get(pos) {
            same += 1;
        } else {
            diff += 1;
        }
    }

    (same,diff)
}

pub fn distance_read_haplo(
    r1: &Frag,
    hap: &FxHashMap<usize, FxHashMap<usize, usize>>,
) -> (usize, usize) {
    let mut diff = 0;
    let mut same = 0;
    for pos in r1.positions.iter() {
        if !hap.contains_key(pos) {
            continue;
        }

        let frag_var = r1.seq_dict.get(pos).unwrap();
        let consensus_var = hap
            .get(pos)
            .unwrap()
            .iter()
            .max_by_key(|entry| entry.1)
            .unwrap()
            .0;
        if *frag_var == *consensus_var {
            same += 1;
        } else {
            let frag_var_count = hap.get(pos).unwrap().get(frag_var);
            if let Some(count) = frag_var_count{
                if count == hap.get(pos).unwrap().get(consensus_var).unwrap(){
                    same += 1;
                    continue
                }
            }
            diff += 1;
        }
    }

    (same, diff)
}

pub fn distance_read_haplo_range(
    r1: &Frag,
    hap: &FxHashMap<usize, FxHashMap<usize, usize>>,
    start : usize,
    end : usize
) -> (usize, usize) {
    let mut diff = 0;
    let mut same = 0;
    for pos in r1.positions.iter() {
        if !hap.contains_key(pos) || *pos > end || *pos < start {
            continue;
        }

        let frag_var = r1.seq_dict.get(pos).unwrap();
        let consensus_var = hap
            .get(pos)
            .unwrap()
            .iter()
            .max_by_key(|entry| entry.1)
            .unwrap()
            .0;
        if *frag_var == *consensus_var {
            same += 1;
        } else {
            diff += 1;
        }
    }

    (same, diff)
}

//Index each position by the set of fragments which overlap that position
pub fn get_all_overlaps(frags: &Vec<Frag>) -> FxHashMap<usize, FxHashSet<&Frag>> {
    let mut overlaps = FxHashMap::default();
    for frag in frags.iter() {
        for pos in frag.positions.iter() {
            let pos_set = overlaps.entry(*pos).or_insert(FxHashSet::default());
            pos_set.insert(frag);
        }
    }

    overlaps
}

//Find all distances between fragments.
//Assumes sorted fragments by first position. 
pub fn get_all_distances(frags: &Vec<Frag>) -> FxHashMap<&Frag, FxHashMap<&Frag, i32>> {
    let mut pairwise_distances = FxHashMap::default();

    for (i, frag1) in frags.iter().enumerate() {
        let mut from_frag_distance = FxHashMap::default();
        for j in i..frags.len() {
            let frag2 = &frags[j];

            if frag1.last_position < frag2.first_position {
                break;
            }

            from_frag_distance.insert(frag2, distance(frag1, frag2).1);
        }
        pairwise_distances.insert(frag1, from_frag_distance);
    }

    pairwise_distances
}

pub fn check_overlap(r1: &Frag, r2: &Frag) -> bool {
    if r1.last_position < r2.first_position {
        return false;
    }
    if r2.last_position < r1.first_position {
        return false;
    }
    let t: Vec<_> = r1.positions.intersection(&r2.positions).collect();
    if t.len() == 0 {
        return false;
    } else {
        return true;
    }
}

pub fn hap_block_from_partition(part: &Vec<FxHashSet<&Frag>>) -> HapBlock {
    let mut block_vec = Vec::new();
    for reads in part.iter() {
        let mut hap_map = FxHashMap::default();
        for frag in reads.iter() {
            for pos in frag.positions.iter() {
                let var_at_pos = frag.seq_dict.get(pos).unwrap();
                let sites = hap_map.entry(*pos).or_insert(FxHashMap::default());
                let site_counter = sites.entry(*var_at_pos).or_insert(0);
                *site_counter += 1;
            }
        }
        block_vec.push(hap_map);
    }
    HapBlock { blocks: block_vec }
}

pub fn get_avg_length(all_frags : &Vec<Frag>, quantile : f64) -> usize{
   let mut length_vec = Vec::new(); 
   for frag in all_frags.iter(){
       length_vec.push(frag.last_position - frag.first_position);
   }
   length_vec.sort();
   return length_vec[(length_vec.len() as f64 * quantile) as usize];
}

pub fn get_length_gn(all_frags : &Vec<Frag>) -> usize{
    let mut last_pos = 0;
    for frag in all_frags.iter(){
        if frag.last_position > last_pos{
            last_pos = frag.last_position;
        }
    }
    last_pos
}

//Get the log p-value for a 1-sided binomial test. This is a asymptotically tight large deviation
//bound. It's super accurate when k/n >> p, but relatively inaccurate when k/n is close to p. One
//super nice thing about this approximation is that it is written as p = exp(A), so log(p) = A
//hence it is extremely numerically stable.
//
//I'm currently using this implementation. We can still mess around with using different approximations.
pub fn stable_binom_cdf_p_rev(n: usize, k: usize, p: f64, div_factor: f64) -> f64 {

    if n == 0 {
        return 0.0;
    }

//    return norm_approx(n,k,p,div_factor);

    let n64 = n as f64;
    let k64 = k as f64;

    //In this case, the relative entropy is bigger than the minimum of 0 which we don't want. 
    let mut a = k64 / n64;
    
    if a == 1.0 {
        //Get a NaN error if a = 1.0;
        a = 0.9999999
    }
    if a == 0.0 {
        //Get a NaN error if we only have errors -- which can happen if we use polishing.
        a = 0.0000001;
    }

    let mut rel_ent = a * (a / p).ln() + (1.0 - a) * ((1.0 - a) / (1.0 - p)).ln();

    //If smaller error than epsilon, invert the rel-ent so that we get a positive probability
    //makes heuristic sense because a smaller than epsilon error is better than an epsilon error
    //for which the relative entropy is 0. 
    if a < p{
        rel_ent = -rel_ent;
    }
    let large_dev_val = -1.0 * n64 / div_factor * rel_ent ;
        //- 0.5 * (6.283*a*(1.0-a)*n64/div_factor).ln();

    return large_dev_val;

    
//    return -1.0 * n64 / div_factor * rel_ent;
}

pub fn log_sum_exp(probs: &Vec<f64>) -> f64{
    let max = probs.iter().copied().fold(f64::NAN, f64::max);
    let mut sum = 0.0;
    for logpval in probs{
        sum += (logpval - max).exp();
    }

    return max + sum.ln();
}
