use crate::types_structs::{build_frag, update_frag, Frag, HapBlock};
use crate::utils_frags;
use bio::alphabets::dna::revcomp;
use bio::io::fastq;
use bio::io::fastq::Writer;
use fxhash::{FxHashMap, FxHashSet};
use rust_htslib::bam::header::Header;
use rust_htslib::bam::HeaderView as HeaderViewBam;
use rust_htslib::bcf::record::GenotypeAllele;
use rust_htslib::{bam, bam::Read as DUMMY_NAME1};
use rust_htslib::{bcf, bcf::Read as DUMMY_NAME2};
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::LineWriter;
use std::io::Write;
use std::io::{self, BufRead};
use std::mem;
use std::path::Path;
use std::str;

// The output is wrapped in a Result to allow matching on errors
// returns an Iterator to the Reader of the lines of the file.
//
fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

// Given a frags.txt file specified as in H-PoP, we return a collection
// (vector) of fragments after processing it.
//
pub fn get_frags_container<P>(filename: P) -> FxHashMap<String, Vec<Frag>>
where
    P: AsRef<Path>,
{
    let mut all_frags = Vec::new();
    let mut counter = 0;

    //Make sure file is able to be read
    if let Ok(lines) = read_lines(filename) {
        for line in lines {
            if let Ok(l) = line {
                let v: Vec<&str> = l.split('\t').collect();

                //First column is the # of blocks
                if let Ok(num_blocks) = v[0].parse::<i32>() {
                    //                    println!("{}",num_blocks);
                    let mut seqs = FxHashMap::default();
                    let mut quals = FxHashMap::default();
                    let mut positions = FxHashSet::default();
                    let mut list_of_positions = Vec::new();
                    let mut first_position = 1;
                    let mut last_position = 1;

                    // For each block, read it into a dictionary with corresp. base
                    for i in 0..num_blocks {
                        let index = i as usize;
                        let start_pos = v[2 * index + 2].parse::<usize>().unwrap();
                        if i == 0 {
                            first_position = start_pos;
                        }
                        for (j, c) in v[2 * index + 3].chars().enumerate() {
                            seqs.insert(start_pos + j, c.to_digit(10).unwrap() as usize);
                            list_of_positions.push(start_pos + j);
                            positions.insert(start_pos + j);
                            last_position = start_pos + j
                        }
                    }

                    let qual_string = v.last().unwrap().as_bytes();
                    for (i, key) in list_of_positions.iter().enumerate() {
                        //We usually have a 33 offset for phred qualities. Rust should throw an
                        //error here if this result is negative.
                        quals.insert(*key, qual_string[i] - 33);
                    }

                    let new_frag = Frag {
                        id: v[1].to_string(),
                        counter_id: counter,
                        seq_dict: seqs,
                        qual_dict: quals,
                        positions: positions,
                        first_position: first_position,
                        last_position: last_position,
                        seq_string: vec![vec![]; 2],
                        qual_string: vec![vec![]; 2],
                        is_paired: false,
                        snp_pos_to_seq_pos: FxHashMap::default(),
                    };

                    all_frags.push(new_frag);
                    counter += 1
                } else {
                    panic!("Not a number found in first column");
                }
            }
        }
    }

    let mut frags_map = FxHashMap::default();
    frags_map.insert(String::from("frag_contig"), all_frags);
    frags_map
}

//Write a vector of blocks into a file.
pub fn write_blocks_to_file<P>(
    out_part_dir: P,
    blocks: &Vec<HapBlock>,
    lengths: &Vec<usize>,
    snp_to_genome: &Vec<usize>,
    part: &Vec<FxHashSet<&Frag>>,
    _first_iter: bool,
    contig: &String,
    break_positions: &FxHashMap<usize, FxHashSet<usize>>,
) where
    P: AsRef<Path>,
{
    let ploidy = blocks[0].blocks.len();
    let filename = out_part_dir
        .as_ref()
        .join(format!("{}_phasing.txt", contig));

    dbg!(&filename);
    let file;
    file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(filename)
        .unwrap();
    //let file = File::create(filename).expect("Can't create file");
    let mut file = LineWriter::new(file);
    let mut length_prev_block = 1;
    let emptydict = FxHashMap::default();
    let unpolished_block = utils_frags::hap_block_from_partition(part);
    //dbg!(snp_to_genome.len(),lengths[0] + 1);

    for (i, block) in blocks.iter().enumerate() {
        let title_string = format!("**{}**\n", contig);
        file.write_all(title_string.as_bytes()).unwrap();
        for pos in length_prev_block..length_prev_block + lengths[i] {
            if break_positions.contains_key(&pos) {
                write!(file, "--------\n").unwrap();
            }
            if snp_to_genome.len() == 0 {
                write!(file, "{}:NA\t", pos).unwrap();
            } else {
                write!(file, "{}:{}\t", pos, snp_to_genome[pos - 1]).unwrap();
            }
            //Write haplotypes
            for k in 0..ploidy {
                let allele_map = block.blocks[k].get(&pos).unwrap_or(&emptydict);
                //If a block has no coverage at a position, we write -1.
                if *allele_map == emptydict {
                    file.write_all(b"-1\t").unwrap();
                } else {
                    let best_allele = allele_map.iter().max_by_key(|entry| entry.1).unwrap().0;
                    write!(file, "{}\t", best_allele).unwrap();
                }
            }

            //Write stats
            for k in 0..ploidy {
                let allele_map_unpolish =
                    unpolished_block.blocks[k].get(&pos).unwrap_or(&emptydict);
                if *allele_map_unpolish == emptydict {
                    write!(file, "NA\t").unwrap();
                } else {
                    let mut first = true;
                    for (site, count) in allele_map_unpolish {
                        if !first {
                            write!(file, "|").unwrap();
                        }
                        if first {
                            first = false;
                        }
                        write!(file, "{}:{}", site, count).unwrap();
                    }
                    write!(file, "\t").unwrap();
                }
            }
            write!(file, "\n").unwrap();
        }
        write!(file, "*****\n").unwrap();
        length_prev_block += lengths[i]
    }
}

//Given a vcf file and a bam file, we get a vector of frags.
pub fn get_frags_from_bamvcf<P>(
    vcf_file: P,
    bam_file: P,
    filter_supplementary: bool,
    use_supplementary: bool,
) -> FxHashMap<String, Vec<Frag>>
where
    P: AsRef<Path>,
{
    //Get which SNPS correspond to which positions on the genome.
    let mut vcf = match bcf::Reader::from_path(vcf_file) {
        Ok(vcf) => vcf,
        Err(_) => panic!("rust_htslib had an error reading the VCF file. Exiting."),
    };
    let mut snp_counter = 1;
    let mut vcf_set_of_pos = FxHashMap::default();
    let mut vcf_pos_allele_map = FxHashMap::default();
    let mut vcf_pos_to_snp_counter_map = FxHashMap::default();
    let mut all_set_of_pos = FxHashSet::default();
    //    let mut set_of_pos = FxHashSet::default();
    //    let mut pos_allele_map = FxHashMap::default();
    //    let mut pos_to_snp_counter_map = FxHashMap::default();
    let vcf_header = vcf.header().clone();

    //    if header.contig_count() > 1 {
    //        panic!("More than 1 contig detected in header of vcf file; please use only 1 contig/reference per vcf file.");
    //    }

    let mut last_ref_chrom: &[u8] = &[];
    for rec in vcf.records() {
        let unr = rec.unwrap();
        let alleles = unr.alleles();
        let mut al_vec = Vec::new();
        let mut is_snp = true;

        let record_rid = unr.rid().unwrap();
        let ref_chrom_vcf = vcf_header.rid2name(record_rid).unwrap();
        //dbg!(String::from_utf8_lossy(ref_chrom_vcf));
        if last_ref_chrom != ref_chrom_vcf {
            snp_counter = 1;
            last_ref_chrom = ref_chrom_vcf;
        }
        let set_of_pos = vcf_set_of_pos
            .entry(ref_chrom_vcf)
            .or_insert(FxHashSet::default());
        let pos_allele_map = vcf_pos_allele_map
            .entry(ref_chrom_vcf)
            .or_insert(FxHashMap::default());
        let pos_to_snp_counter_map = vcf_pos_to_snp_counter_map
            .entry(ref_chrom_vcf)
            .or_insert(FxHashMap::default());

        for allele in alleles.iter() {
            if allele.len() > 1 {
                is_snp = false;
                break;
            }
            al_vec.push(allele[0]);
        }

        if !is_snp {
            //            println!(
            //                "BAM : Variant at position {} is not a snp. Ignoring.",
            //                unr.pos()
            //            );
            continue;
        }

        set_of_pos.insert(unr.pos());
        all_set_of_pos.insert(unr.pos());
        pos_to_snp_counter_map.insert(unr.pos(), snp_counter);
        snp_counter += 1;
        pos_allele_map.insert(unr.pos(), al_vec);
    }

    //    let mut bam = bam::Reader::from_path(bam_file).unwrap();
    let mut bam = match bam::Reader::from_path(bam_file) {
        Ok(bam) => bam,
        Err(_) => panic!("rust_htslib had an error while reading the BAM file. Exiting"),
    };

    //Check the headers to see how many references there are.
    let header = Header::from_template(bam.header());
    let bam_header_view = HeaderViewBam::from_header(&header);

    //This may be important : We assume that distinct reads have different names. I can see this
    //being a problem in some weird bad cases, so be careful.
    let mut ref_id_to_frag = FxHashMap::default();
    let mut counter_id = 0;

    //Scan the pileup table for every position on the genome which contains a SNP to get the aligned reads corresponding to the SNP. TODO : There should be a way to index into the bam.pileup() object so we don't have to iterate through positions which we already know are not SNPs.
    let errors_mask = 1796;
    let first_in_pair_mask = 64;
    let second_in_pair_mask = 128;
    let secondary_mask = 256;
    let supplementary_mask = 2048;
    let mapq_supp_cutoff = 59;
    let mapq_normal_cutoff = 15;
    for p in bam.pileup() {
        let pileup = p.unwrap();
        let pos_genome = pileup.pos();

        if !all_set_of_pos.contains(&(pos_genome as i64)) {
            continue;
        }

        for alignment in pileup.alignments() {
            if !alignment.is_del() && !alignment.is_refskip() {
                let aln_record = alignment.record();
                let flags = aln_record.flags();
                let mapq = aln_record.mapq();
                let is_paired;
                let mut pair_number = 0;

                if flags & first_in_pair_mask > 0 {
                    is_paired = true;
                } else if flags & second_in_pair_mask > 0 {
                    is_paired = true;
                    pair_number = 1;
                } else {
                    is_paired = false;
                }

                if mapq < mapq_normal_cutoff {
                    continue;
                }

                //Erroneous alignment, skip
                if flags & errors_mask > 0 {
                    //dbg!(&flags,&id_string);
                    continue;
                }

                //Secondary alignment, skip
                if flags & secondary_mask > 0 {
                    //dbg!(&flags,&id_string);
                    continue;
                }

                let is_supp;
                if flags & supplementary_mask > 0 {
                    is_supp = true;
                    if !use_supplementary {
                        continue;
                    }
                    if filter_supplementary {
                        if mapq < mapq_supp_cutoff {
                            continue;
                        }
                    }
                } else {
                    is_supp = false;
                }

                //                println!("{}-{}-{}",&alignment.record().seq().len(), flags , &id_string);
                //

                let tid = aln_record.tid();
                let ref_chrom = bam_header_view.tid2name(tid as u32);
                //dbg!(String::from_utf8_lossy(ref_chrom));
                let get_ref_chrom = vcf_pos_to_snp_counter_map.get(ref_chrom);

                let pos_to_snp_counter_map = match get_ref_chrom {
                    Some(pos_to_snp_counter_map) => pos_to_snp_counter_map,
                    None => continue,
                };

                if pos_to_snp_counter_map.contains_key(&(pos_genome as i64)) == false {
                    continue;
                }
                let id_string = String::from_utf8(aln_record.qname().to_vec()).unwrap();
                let id_to_frag = ref_id_to_frag
                    .entry(ref_chrom)
                    .or_insert(FxHashMap::default());

                let snp_id = pos_to_snp_counter_map.get(&(pos_genome as i64)).unwrap();
                let id_string2 = id_string.clone();

                if !id_to_frag.contains_key(&id_string) {
                    counter_id += 1;
                }

                let readbase = alignment.record().seq()[alignment.qpos().unwrap()];
                let qualbase = alignment.record().qual()[alignment.qpos().unwrap()];

                for (i, allele) in vcf_pos_allele_map
                    .get(ref_chrom)
                    .unwrap()
                    .get(&(pos_genome as i64))
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    //Only build the frag if the base is one of the SNP alleles.
                    if readbase == *allele {
                        let mut frag;
                        if id_to_frag.contains_key(&id_string2) {
                            frag = id_to_frag.get_mut(&id_string2).unwrap();
                        } else {
                            frag = id_to_frag
                                .entry(id_string2)
                                .or_insert(build_frag(id_string, counter_id, is_paired));
                        }
                        //                        let mut frag = id_to_frag.entry(id_string2).or_insert(build_frag(
                        //                            id_string,
                        //                            counter_id,
                        //                            supp_aln,
                        //                            alignment.record().seq().as_bytes(),
                        //                            alignment.record().qual().to_vec(),
                        //                        ));
                        update_frag(
                            &mut frag,
                            i,
                            *snp_id,
                            qualbase,
                            pair_number,
                            is_supp,
                            &aln_record,
                            alignment.qpos().unwrap(),
                        );
                        break;
                    }
                }
            }
        }
    }

    let mut ref_vec_frags = FxHashMap::default();
    let mut keys = FxHashSet::default();
    for ref_chrom in ref_id_to_frag.keys() {
        ref_vec_frags.insert(String::from_utf8(ref_chrom.to_vec()).unwrap(), Vec::new());
        keys.insert(ref_chrom.clone());
    }
    for ref_chrom in keys {
        let vec_frags = ref_vec_frags
            .get_mut(&String::from_utf8(ref_chrom.to_vec()).unwrap())
            .unwrap();
        let id_to_frag = ref_id_to_frag.get_mut(ref_chrom).unwrap();
        let id_to_frag = mem::replace(id_to_frag, FxHashMap::default());
        for (_id, frag) in id_to_frag.into_iter() {
            //IMPORTANT: I'm turning this off for metagenomics because some fragments may only
            //index one read. However, this is still useful because we don't know ploidy info.
            let mut prev_pos = frag.first_position;
            for pos in frag.first_position + 1..frag.last_position {
                if !frag.positions.contains(&pos) && (pos - prev_pos < 100) {
                    //random number. TODO TESTING GAPS IN FRAGMENTS
                    //                                        frag.seq_dict.insert(pos, 9);
                    //                                        frag.qual_dict.insert(pos, 7);
                    //                                        frag.positions.insert(pos);
                }
                prev_pos = pos;
            }
            if frag.positions.len() > 0 {
                vec_frags.push(frag);
            }
        }
    }

    //Think this was for debugging?
    for vec in ref_vec_frags.values() {
        for frag in vec.iter() {
            let mut vec_snp_seq: Vec<(usize, usize)> = frag
                .snp_pos_to_seq_pos
                .iter()
                .map(|(x, y)| (*x, y.1))
                .collect();
            vec_snp_seq.sort();
            let mut prev_seq_pos = 0;
            for item in vec_snp_seq.iter() {
                if prev_seq_pos > item.1 {
                    //dbg!(&vec_snp_seq, &frag.id);
                    //                    panic!();
                }
                prev_seq_pos = item.1;
            }
        }
    }

    ref_vec_frags
}

//Read a vcf file to get the genotypes. We read genotypes into a dictionary of keypairs where the
//keys are positions, and the values are dictionaries which encode the genotypes. E.g. the genotype
//1 1 0 0 at position 5 would be (5,{1 : 2, 0 : 2}).
pub fn get_genotypes_from_vcf_hts<P>(
    vcf_file: P,
) -> (
    FxHashMap<String, Vec<usize>>,
    FxHashMap<String, FxHashMap<usize, FxHashMap<usize, usize>>>,
    usize,
)
where
    P: AsRef<Path>,
{
    let mut vcf = match bcf::Reader::from_path(vcf_file) {
        Ok(vcf) => vcf,
        Err(_) => panic!("rust_htslib had an error while reading the BAM file. Exiting."),
    };
    let mut map_positions_vec = FxHashMap::default();
    let mut map_genotype_dict = FxHashMap::default();
    //let mut positions_vec = Vec::new();
    //let mut genotype_dict = FxHashMap::default();
    let header = vcf.header().clone();
    let mut snp_counter = 1;
    let mut vcf_ploidy = 0;

    if header.sample_count() > 1 {
        panic!("More than 1 sample detected in header of vcf file; please use only 1 sample");
    }

    //    if header.contig_count() > 1 {
    //        panic!("More than 1 contig detected in header of vcf file; please use only 1 contig/reference per vcf file.");
    //    }

    let mut last_ref_chrom: &[u8] = &[];

    for rec in vcf.records() {
        let unr = rec.unwrap();
        let alleles = unr.alleles();
        let mut is_snp = true;
        let record_rid = unr.rid().unwrap();
        let ref_chrom_vcf = header.rid2name(record_rid).unwrap();
        if last_ref_chrom != ref_chrom_vcf {
            last_ref_chrom = ref_chrom_vcf;
            snp_counter = 1;
        }

        for allele in alleles.iter() {
            if allele.len() > 1 {
                is_snp = false;
                break;
            }
        }

        if !is_snp {
            //            println!(
            //                "VCF : Variant at position {} is not a snp. Ignoring.",
            //                unr.pos()
            //            );
            continue;
        }

        if let Ok(_) = unr.genotypes() {
            let genotypes = unr.genotypes().unwrap().get(0);
            vcf_ploidy = genotypes.len();
            let mut genotype_counter = FxHashMap::default();
            for allele in genotypes.iter() {
                match allele {
                    GenotypeAllele::Unphased(x) => {
                        let count = genotype_counter.entry(*x as usize).or_insert(0);
                        *count += 1
                    }
                    GenotypeAllele::Phased(x) => {
                        let count = genotype_counter.entry(*x as usize).or_insert(0);
                        *count += 1
                    }
                    GenotypeAllele::UnphasedMissing => {
                        for i in 0..4 {
                            let count = genotype_counter.entry(i).or_insert(0);
                            *count += 1;
                        }
                    }
                    GenotypeAllele::PhasedMissing => {
                        for i in 0..4 {
                            let count = genotype_counter.entry(i).or_insert(0);
                            *count += 1;
                        }
                    }
                }
            }
            let genotype_dict = map_genotype_dict
                .entry(String::from_utf8(ref_chrom_vcf.to_vec()).unwrap())
                .or_insert(FxHashMap::default());
            genotype_dict.insert(snp_counter, genotype_counter);
        }

        let positions_vec = map_positions_vec
            .entry(String::from_utf8(ref_chrom_vcf.to_vec()).unwrap())
            .or_insert(Vec::new());
        //+1 because htslib is 0 index by default
        positions_vec.push(unr.pos() as usize + 1);
        snp_counter += 1;
    }

    (map_positions_vec, map_genotype_dict, vcf_ploidy)
}

//Convert a fragment which stores sequences in a dictionary format to a block format which makes
//writing to frag files easier.
fn convert_dict_to_block(frag: Frag) -> (Vec<usize>, Vec<Vec<usize>>, Vec<u8>) {
    let d = frag.seq_dict;
    let vec_d: BTreeMap<usize, usize> = d.into_iter().collect();
    let vec_q: BTreeMap<usize, u8> = frag.qual_dict.into_iter().collect();
    let mut prev_pos = 0;
    let mut block_start_pos = Vec::new();
    let mut blocks = Vec::new();
    let mut block = Vec::new();
    let mut qual_block = Vec::new();

    for (pos, var) in &vec_d {
        if prev_pos == 0 {
            prev_pos = *pos;
            block.push(*var);
            block_start_pos.push(*pos);
        } else if pos - prev_pos > 1 {
            blocks.push(block);
            block = vec![*var];
            block_start_pos.push(*pos);
            prev_pos = *pos;
        } else if pos - prev_pos == 1 {
            block.push(*var);
            prev_pos = *pos;
        }
    }

    for (_pos, q) in &vec_q {
        qual_block.push(*q);
    }

    blocks.push(block);
    (block_start_pos, blocks, qual_block)
}

//Write a vector of sorted fragment files by first position (no guarantees on end position) to a
//file in the same format as H-PoP and other haplotypers.
pub fn write_frags_file(frags: Vec<Frag>, filename: String) {
    let file = File::create(filename).expect("Can't create file");
    let mut file = LineWriter::new(file);
    for frag in frags.into_iter() {
        let frag_id = frag.id.clone();
        let (start_vec, blocks, qual_block) = convert_dict_to_block(frag);
        if start_vec.len() != blocks.len() {
            dbg!(start_vec.len(), blocks.len());
            panic!("Block length diff");
        }

        write!(file, "{}\t", blocks.len()).unwrap();
        write!(file, "{}\t", frag_id).unwrap();
        for i in 0..blocks.len() {
            write!(file, "{}\t", start_vec[i]).unwrap();
            for var in blocks[i].iter() {
                write!(file, "{}", *var).unwrap();
            }
            write!(file, "\t").unwrap();
        }

        for q in qual_block.iter() {
            if *q as usize + 33 > 255 {
                write!(file, "{}", (*q) as char).unwrap();
            } else {
                write!(file, "{}", (*q + 33) as char).unwrap();
            }
        }

        write!(file, "\n").unwrap();
    }
}

pub fn write_output_partition_to_file(
    part: &Vec<FxHashSet<&Frag>>,
    snp_range_parts_vec: Vec<(usize, usize)>,
    out_bam_part_dir: String,
    contig: &String,
    snp_pos_to_genome_pos: &Vec<usize>,
) {
    fs::create_dir_all(&out_bam_part_dir).unwrap();
    fs::create_dir_all(&format!("{}/local_parts", out_bam_part_dir)).unwrap();
    fs::create_dir_all(&format!("{}/short_reads", out_bam_part_dir)).unwrap();
    fs::create_dir_all(&format!("{}/long_reads", out_bam_part_dir)).unwrap();
    fs::create_dir_all(&format!("{}/haplotypes", out_bam_part_dir)).unwrap();

    let contig_path = &format!("{}/{}_part.txt", out_bam_part_dir, contig);
    //        out_bam_part_dir
    //        .as_ref()
    //        .join(format!("{}_part.txt", contig));
    let file = File::create(contig_path).expect("Can't create file");
    let mut file = LineWriter::new(file);

    for (i, set) in part.iter().enumerate() {
        let mut vec_part: Vec<&&Frag> = set.into_iter().collect();
        vec_part.sort_by(|a, b| a.first_position.cmp(&b.first_position));
        write!(file, "#{}\n", i).unwrap();

        //Non-empty means that we're writing the final partition after path collection
        if !snp_range_parts_vec.is_empty() {
            let append;
            if i == 0 {
                append = true;
            } else {
                append = false;
            }
            write_fragset_haplotypes(
                set,
                &format!("{}", i),
                &out_bam_part_dir,
                &snp_pos_to_genome_pos,
                append,
            );

            let part_fastq_reads = format!("{}/long_reads/{}_part.fastq", out_bam_part_dir, i);
            let part_fastq_reads_paired1 =
                format!("{}/short_reads/{}_part_paired1.fastq", out_bam_part_dir, i);
            let part_fastq_reads_paired2 =
                format!("{}/short_reads/{}_part_paired2.fastq", out_bam_part_dir, i);

            let fastq_file = File::create(part_fastq_reads).expect("Can't create file");
            let fastq_file1 = File::create(part_fastq_reads_paired1).expect("Can't create file");
            let fastq_file2 = File::create(part_fastq_reads_paired2).expect("Can't create file");

            let mut fastq_writer = fastq::Writer::new(fastq_file);
            let mut fastq_writer_paired1 = fastq::Writer::new(fastq_file1);
            let mut fastq_writer_paired2 = fastq::Writer::new(fastq_file2);

            //1-indexing for snp position already accounted for
            let left_snp_pos = snp_range_parts_vec[i].0;
            let right_snp_pos = snp_range_parts_vec[i].1;
            let extension = 25;
            for frag in vec_part.iter() {
                let mut found_primary = false;
                for seq in frag.seq_string.iter() {
                    if seq.len() != 0 {
                        found_primary = true;
                        break;
                    }
                }
                if !found_primary {
                    //                    println!(
                    //                        "{} primary not found. Paired: {}",
                    //                        &frag.id, &frag.is_paired
                    //                    );
                    continue;
                }
                //Circularity weirdness. Throw away these reads, unfortunately, for now.
                if frag.first_position > right_snp_pos {
                    continue;
                }
                if frag.last_position < left_snp_pos {
                    continue;
                }
                let mut left_seq_pos;
                let mut tmp = left_snp_pos;
                let left_read_pair;
                loop {
                    if frag.snp_pos_to_seq_pos.contains_key(&tmp) {
                        let info = frag.snp_pos_to_seq_pos[&tmp];
                        left_seq_pos = info.1;
                        left_read_pair = info.0;
                        break;
                    }
                    tmp += 1;
                    if tmp - left_snp_pos > 10000000 {
                        dbg!(
                            &frag.first_position,
                            &frag.last_position,
                            left_snp_pos,
                            right_snp_pos,
                            &frag.snp_pos_to_seq_pos,
                        );
                        panic!();
                    }
                }
                if left_seq_pos > extension {
                    left_seq_pos -= extension;
                } else {
                    left_seq_pos = 0;
                }

                let mut right_seq_pos;
                let mut tmp = right_snp_pos;
                let right_read_pair;
                loop {
                    if frag.snp_pos_to_seq_pos.contains_key(&tmp) {
                        let info = frag.snp_pos_to_seq_pos[&tmp];
                        right_seq_pos = info.1;
                        right_read_pair = info.0;
                        break;
                    }
                    if tmp == 0 {
                        dbg!(&frag.positions, left_snp_pos, right_snp_pos);
                    }
                    tmp -= 1;
                }

                if right_seq_pos < frag.seq_string[right_read_pair as usize].len() - extension - 1 {
                    right_seq_pos += extension;
                } else {
                    right_seq_pos = frag.seq_string[right_read_pair as usize].len() - 1;
                }

                if frag.is_paired {
                    write_paired_reads_no_trim(
                        &mut fastq_writer_paired1,
                        &mut fastq_writer_paired2,
                        left_read_pair,
                        right_read_pair,
                        left_seq_pos,
                        right_seq_pos,
                        &frag,
                    );
                } else {
                    if left_seq_pos > right_seq_pos {
                        println!(
                            "{} left seq pos > right seq pos at {:?}",
                            &frag.id, snp_range_parts_vec[i]
                        );
                        continue;
                    }
                    fastq_writer
                        .write(
                            &frag.id,
                            None,
                            &frag.seq_string[0].as_slice()[left_seq_pos..right_seq_pos + 1],
                            &frag.qual_string[0].as_slice()[left_seq_pos..right_seq_pos + 1],
                        )
                        .unwrap();
                }
            }
        }

        for frag in vec_part {
            write!(
                file,
                "{}\t{}\t{}\n",
                frag.id.clone(),
                frag.first_position,
                frag.last_position
            )
            .unwrap();
        }
    }
}

fn write_paired_reads_no_trim<W: Write>(
    fastq_writer_paired1: &mut Writer<W>,
    fastq_writer_paired2: &mut Writer<W>,
    _left_read_pair: u8,
    _right_read_pair: u8,
    _left_seq_pos: usize,
    _right_seq_pos: usize,
    frag: &Frag,
) {
    if frag.seq_string[0].len() == 0 {
        fastq_writer_paired1
            .write(
                &format!("{}/1", frag.id),
                None,
                //Write N instead
                &vec![78],
                &vec![20],
            )
            .unwrap();
    } else {
        fastq_writer_paired1
            .write(
                &format!("{}/1", frag.id),
                None,
                &frag.seq_string[0],
                &frag.qual_string[0],
            )
            .unwrap();
    }
    if frag.seq_string[1].len() == 0 {
        fastq_writer_paired2
            .write(
                &format!("{}/2", frag.id),
                None,
                //Write N instead
                &vec![78],
                &vec![20],
            )
            .unwrap();
    } else {
        fastq_writer_paired2
            .write(
                &format!("{}/2", frag.id),
                None,
                &revcomp(&frag.seq_string[1]),
                &frag.qual_string[1],
            )
            .unwrap();
    }
}

fn _write_paired_reads<W: Write>(
    fastq_writer_paired1: &mut Writer<W>,
    fastq_writer_paired2: &mut Writer<W>,
    left_read_pair: u8,
    right_read_pair: u8,
    left_seq_pos: usize,
    right_seq_pos: usize,
    frag: &Frag,
) {
    if left_read_pair == right_read_pair {
        let writer;
        let other_writer;
        let read_pair;
        let other_read_pair;
        if left_read_pair == 0 {
            read_pair = 0;
            other_read_pair = 1;
            writer = fastq_writer_paired1;
            other_writer = fastq_writer_paired2;
        } else {
            read_pair = 1;
            other_read_pair = 0;
            writer = fastq_writer_paired2;
            other_writer = fastq_writer_paired1;
        }
        writer
            .write(
                &format!("{}/{}", frag.id, read_pair),
                None,
                &frag.seq_string[read_pair as usize].as_slice()[left_seq_pos..right_seq_pos + 1],
                &frag.qual_string[read_pair as usize].as_slice()[left_seq_pos..right_seq_pos + 1],
            )
            .unwrap();
        other_writer
            .write(
                &format!("{}/{}", frag.id, other_read_pair),
                None,
                //Write N instead
                &vec![78],
                &vec![20],
            )
            .unwrap();
    } else {
        if frag.seq_string[left_read_pair as usize].len() == 0 {
            fastq_writer_paired1
                .write(
                    &format!("{}/1", frag.id),
                    None,
                    //Write N instead
                    &vec![78],
                    &vec![20],
                )
                .unwrap();
        } else {
            fastq_writer_paired1
                .write(
                    &format!("{}/1", frag.id),
                    None,
                    &frag.seq_string[left_read_pair as usize].as_slice()[left_seq_pos..],
                    &frag.qual_string[left_read_pair as usize].as_slice()[left_seq_pos..],
                )
                .unwrap();
        }
        if frag.seq_string[right_read_pair as usize].len() == 0 {
            fastq_writer_paired2
                .write(
                    &format!("{}/2", frag.id),
                    None,
                    //Write N instead
                    &vec![78],
                    &vec![20],
                )
                .unwrap();
        } else {
            let qual_cut_string =
                &frag.qual_string[right_read_pair as usize].as_slice()[..right_seq_pos];
            let rev_quals: Vec<u8> = qual_cut_string.into_iter().rev().map(|x| *x).collect();
            fastq_writer_paired2
                .write(
                    &format!("{}/2", frag.id),
                    None,
                    &revcomp(
                        &frag.seq_string[right_read_pair as usize].as_slice()[..right_seq_pos],
                    ),
                    //TODO Do we need to flip this as well?
                    rev_quals.as_slice(),
                )
                .unwrap();
        }
    }
}

pub fn alignment_passed_check(
    flags: u16,
    mapq: u8,
    use_supplementary: bool,
    filter_supplementary: bool,
) -> (bool, bool) {
    let errors_mask = 1796;
    let secondary_mask = 256;
    let supplementary_mask = 2048;
    let mapq_supp_cutoff = 59;
    let mapq_normal_cutoff = 15;

    let is_supp;
    if flags & supplementary_mask > 0 {
        is_supp = true;
        if !use_supplementary {
            return (false, true);
        }
        if filter_supplementary {
            if mapq < mapq_supp_cutoff {
                return (false, true);
            }
        }
    } else {
        is_supp = false;
    }

    if mapq < mapq_normal_cutoff {
        return (false, is_supp);
    }
    //Erroneous alignment, skip
    if flags & errors_mask > 0 {
        //dbg!(&flags,&id_string);
        return (false, is_supp);
    }

    //Secondary alignment, skip
    if flags & secondary_mask > 0 {
        //dbg!(&flags,&id_string);
        return (false, is_supp);
    }

    return (true, is_supp);
}

fn write_fragset_haplotypes(
    frags: &FxHashSet<&Frag>,
    name: &str,
    dir: &str,
    snp_pos_to_genome_pos: &Vec<usize>,
    append: bool,
) {
    let filename = format!("{}/haplotypes/{}_hap.txt", dir, name);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .open(filename)
        .unwrap();

    let hap_map = utils_frags::set_to_seq_dict(&frags);
    let emptydict = FxHashMap::default();
    let title_string = format!(">{}\n", name);
    write!(file, "{}", title_string).unwrap();
    let mut positions: Vec<&usize> = hap_map.keys().collect();
    if positions.len() == 0{
        return
    }
    positions.sort();
    for pos in *positions[0]..*positions[positions.len() - 1] {
        if snp_pos_to_genome_pos.len() == 0 {
            write!(file, "{}:NA\t", pos).unwrap();
        } else {
            write!(file, "{}:{}\t", pos, snp_pos_to_genome_pos[pos - 1]).unwrap();
        }
        let allele_map = hap_map.get(&pos).unwrap_or(&emptydict);
        //If a block has no coverage at a position, we write -1.
        if *allele_map == emptydict {
            file.write_all(b"-1\t").unwrap();
        } else {
            let best_allele = allele_map.iter().max_by_key(|entry| entry.1).unwrap().0;
            write!(file, "{}\t", best_allele).unwrap();
        }

        if *allele_map == emptydict {
            write!(file, "NA\t").unwrap();
        } else {
            let mut first = true;
            for (site, count) in allele_map {
                if !first {
                    write!(file, "|").unwrap();
                }
                if first {
                    first = false;
                }
                write!(file, "{}:{}", site, count).unwrap();
            }
            write!(file, "\t").unwrap();
        }
        write!(file, "\n").unwrap();
    }
}
