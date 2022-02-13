# glopp (name to be decided) : polyploid phasing from read sequencing

## Introduction

**glopp** is a software package for single individual haplotype phasing of polyploid organisms from read sequencing. 

Given 

1. a list of variants in .vcf format
2. a set of reads mapped to a reference in .bam format

**glopp** performs strain/haplotype phasing.

### Requirements 

1. [rust](https://www.rust-lang.org/tools/install) and associated tools such as cargo are required and assumed to be in PATH.
2. [cmake](https://cmake.org/download/) version > 3.12 is required. It's sufficient to download the binary from the link and do `PATH="/path/to/cmake-3.xx.x-linux-x86_64/bin/:$PATH"` before installation. 
### Install

```
git clone https://github.com/bluenote-1577/glopp
cd glopp
#git checkout flow <- development branch.
cargo build --release
./target/release/glopp -h
```

`cargo build --release` builds the **glopp** binary, which is found in the ./target/release/ directory. 

## Using glopp

```
glopp -b bamfile.bam -c vcffile.vcf -o output_dir #long-read assuming ~10kb average length, 10% error rates
glopp -b bamfile.bam -c vcffile.vcf -o output_dir -e 0.005 -l 500 #short-read assuming 150x2 bp, low error rates

```

The standard mode of usage is to specify a bam file using the option **-b** and a vcf file using the option **-c**. The output is written to folder with value of option **-o**. 

**VCF File:** glopp currently only uses SNP information and does not take into account indels. VCF file must have valid contig headers -- see the Misc section if your VCF does not have valid contig headers.

**BAM File:** the bam file may contain multiple contigs/references which the reads are mapped to as long as the corresponding contigs also appear in the vcf file.

For a quick test, we provide a VCF and BAM files in the tests folder. Run
```
 ./target/release/glopp -b tests/test_bams/pds_ploidy3.bam -c tests/test_vcfs/pds.vcf -o results
```
to run glopp on a 3 Mb section of a simulated 3x ploidy potato chromosome with 30x read coverage.

## Output

```
results
│      
└───contig1_in_bam
│   │   all_part.txt
│   │   pet_graph.dot
|   |   (other debug files)
│   │
│   └───haplotypes
│   |   │   0_hap.txt
│   |   │   1_hap.txt
│   |   │   ...
|   |
│   └───long_reads
│   |   │   0_part.fastq
│   |   |   ...
|   |
│   └───short_reads
│   |   │   0_part_paired1.fastq
│   |   |   0_part_paired2.fastq
|   |
|   └───(debug_folders)
└───contig2_in_bam
    │   ...
    │   ...
```
glopp outputs a set of **haplotigs**. We define a haplotig to be a set of reads that belong to the same strain. The collection of all haplotigs is found in the `results/contig1/all_part.txt` file. 

The following information is output for each haplotig:

1. a **haplotype** which is the sequence of SNPs on each haplotig in the `haplotypes` folder. 
2. trimmed long-reads (if using long-reads) corresponding to each haplotig are found in the `long_reads` folder. 
3. trimmed short-reads (if using short-reads) corresponding to each haplotig are found in the `short_reads` folder. 

### Haplotigs ``results/contig/all_part.txt`` 

Each haplotig corresponds to a cluster of reads and is presented in the following format:

```
#0 (haplotig #0)
(read_name1) (first SNP position covered) 
(read_name2) (first SNP position covered)
...
#1 (haplotig #1)
...
```

### Haplotype output ``results/contig/haplotypes/``
For each haplotig, glopp outputs a haplotype file `#_hap.txt` in the following format:

```
>(haplotig number)
(snp #1):(genome position)     (consensus allele #: 0/1/2...)    (allele #1):(support)|(allele #2):(support)|...
(snp #2):(genome position)     (consensus allele #: 0/1/2...)    (allele #1):(support)|(allele #2):(support)|...
...

```

1. Col. 1 indicates the # and position of the SNP. 
2. Col. 2 states which allele is the consensus allele.
3. Col. 3 describes how many reads support each allele (i.e. how many reads in the haplotig have the allele at the SNP position). 

### Read output ``results/contig/*_reads/``

The reads in each haplotig can be found in either the `long_reads` or `short_reads` folder, depending on which type of read is used. Note that fastq files in these folders are trimmed and thus differ from the original reads. This is done so that all reads in a haplotig fall within an interval on the genome and do not extend past the interval. 

### Debugging

Extra debug files in `local_parts` and `debug_paths` show the local partitions and the path corresponding to the haplotigs and local partitions. To visualize the flow-graph constructed, a graphviz `pet_graph.dot` file is included. If graphviz is installed, this can be visualized by running `dot -Tps results/contig/pet_graph.dot -o outfile.ps` and looking at the resulting `outfile.ps`. 

## Assembling output reads in `results/contig/*_reads/`

If you want to assemble the haplotigs (in the same way strainberry does) then the utility scripts `strains_phase_scripts/assemble_from_glopp_out.py` or `assemble_shortreads_from_glopp_out.py` for long and short reads respectively allow you to do so. Ensure that 

1. wtdbg2 (for long-reads)
2. minimap2
3. abyss (for short-reads)

and associated binaries are present in path or edit the files to include paths to the binaries. Run 

```strains_phase_scripts/assemble_from_glopp_out.py results_dir results_dir reference.fa```

where `results_dir` is the output of glopp (i.e. the `-o` output) for long-reads and the other script for short reads. 

All haplotigs will be assembled in `results_dir/intermediate` and all assembled haplotigs will be mapped onto `reference.fa` in the `results_dir/all_assemblies.bam` bam file. 

**NOTE**: this only works for single contig phasings, i.e. your bam file is only aligned to a single reference. I will test this later for multi-contig phasings.


## Extra scripts

### VCF requires contig headers
We found that some variant callers don't put contig headers in the VCF file. In this situation, run `python scripts/write_contig_headers_vcf.py (vcf_file)` to get a new VCF with contig headers.

### Output BAM partition
To get a set of BAM files which correspond to each haplotig, use

``python scripts/get_bam_partition.py results/contig/all_part.txt used_bam_file.bam  -prefix``

This will output a set of bams labelled `prefix1.bam`, `prefix2.bam` and so forth for each haplotig. This script requires pysam. 

### Manually consensus for testing

Suppose you already have a partitioning of reads. That is, you have bam files `bam_file1, bam_file2, bam_file3` and you want to use this partioning for the phasing. Use the `consensus` binary to get a phasing from the .bam files by `consensus -v (vcf_file) -b (bam_file1) (bam_file2) (bam_file3) -o (consensus_file.txt)`. This is useful if you have synthetic data. 

