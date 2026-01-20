# rapid-map
A fast, configurable logical to physical memory mapping tool for FPGA architectures. Takes in a set of circuits as blocks, and their logical RAMs, and selects efficient physical implementations based on the given memory block parameters. The program calculates and optimizes area as geometric-mean across all circuits. 

## Features
The input memory instance must be defined as 
* ROM: Uses only one port and is never written to. 
* SinglePort: Uses only one port, as a r/w port.
* SimpleDualPort: Uses 1r port and 1w port. 
*TrueDualPort: Uses two r/w ports to do 1r and 1w, 2w, or 2r each cycle. 

## Supported Physical Implementations
The mapper can target up to three physical memory types 
* LUTRAM: A LUT-based small memory model. 
* 8K BRAM: Configurable capacity and logic-block spacing (default capacity is 8192 bits and spacing is 8K/10-LUTs).
* 128K BRAM: Configuratble capacity and logic-block spacing (default capacity is 131072 bits and spacing is 128K/300LUTs). 

## Configuration
Architecture settings can be overridden using -p 
```bash
cargo run --release -- -p <has_lutram> <lutram_fraction> <has_ram1> <ram1_bits> <lbs_per_ram1> <max_width_ram1> <has_ram2> <ram2_bits> <lbs_per_ram2> <max_width_ram2>
```
or 
```bash
target/debug/ram_mapper -p true 0.75 true 8192 10 32 true 131072 300 128
```
* has_lutram: true/false or 1/0
* lutram_fraction: Fraction of logic blocks that can be LUTRAM-capable (0..1)
* has_ram1: Enable 1st BRAM
* ram1_bits: Capacity in bits for 1st memory block (default 8192)
* lbs_per_ram1: Logic-block spacing for the 1st memory block (default 10 LUTs)
* max_width_ram1: Maximum supported with non-TDP (default is 32 bits). For TDP, maximum width is max_width/2
* ram2_bits: Capacity in bits for 2nd memory block (default 131072)
* lbs_per_ram2: Logic-block spacing for the 2nd memory block (default 300 LUTs)
* max_width_ram2: Maximum supported with non-TDP (default is 128 bits)

Atleast of LUTRAM/RAM1/RAM2 must be enabled. 

Examples:
```bash
cargo run --release
```
```bash
cargo run --release -- -p false 0.5 true 8192 10 32 true 131072 300 128
```
```bash
cargo run --release -- -p false 0.5 true 16384 20 64 true 65536 200 64
```


