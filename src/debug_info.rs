use crate::{debugger::catch_syscall, mem, syscall::SyscallStack};
#[allow(unused)]
use gimli::{
    self,
    read::{AttributeValue, AttrsIter, DieReference, EvaluationResult},
    DebugLineOffset, Dwarf, EndianSlice, Reader, RunTimeEndian,
};
use nix::{
    sys::wait::{waitpid, WaitPidFlag, WaitStatus},
    unistd::Pid,
};
use object::{Object, ObjectSection, ObjectSymbol};
use proc_maps::MapRange;
use std::{
    borrow::{self, Cow},
    fs, io,
};

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub name: String,
    pub offset: u64,
}

#[derive(Debug, Clone)]
pub struct VariableInfo {
    pub name: String,
    pub offset: u64,
}

impl VariableInfo {
    pub fn is_included(&self, map: &MapRange, base_addr: u64) -> bool {
        let map_offset = map.offset as u64;
        let map_size = map.size() as u64;
        let map_start = map.start() as u64;

        let base_diff = map_start - base_addr;
        let var_offset = if self.offset > base_diff {
            self.offset - base_diff
        } else {
            self.offset
        };
        (map_offset <= var_offset) && (var_offset < (map_offset + map_size))
    }
}

#[derive(Debug, Clone)]
pub struct TdbDebugInfo {
    pub fn_info_vec: Vec<FunctionInfo>,
    pub var_info_vec: Vec<VariableInfo>,
    pub mmap_info_vec: Vec<MapRange>,
    pub base_addr: u64,
}

impl TdbDebugInfo {
    pub fn init(filename: &str, pid: Pid, syscall_stack: &mut SyscallStack) -> (Self, WaitStatus) {
        let file = fs::File::open(filename).unwrap();
        let mmap = unsafe { memmap::Mmap::map(&file).unwrap() };
        let object = object::File::parse(&*mmap).unwrap();

        let mut fn_info_vec = Vec::new();
        let mut var_info_vec = Vec::new();

        Self::get_elf_fn_info(&object, &mut fn_info_vec);
        Self::get_elf_var_info(&object, &mut var_info_vec);

        let (mmap_info_vec, status) = Self::get_mmap_info_vec(pid, filename, syscall_stack);

        let mut base_addr = u64::MAX;
        for m in &mmap_info_vec {
            if (m.start() as u64) < base_addr {
                base_addr = m.start() as u64;
            }
        }

        (
            Self {
                fn_info_vec,
                var_info_vec,
                mmap_info_vec,
                base_addr,
            },
            status,
        )
    }

    pub fn get_breakpoint_offset(&self, bp_symbol_name: &str) -> Option<u64> {
        for f in &self.fn_info_vec {
            if f.name == bp_symbol_name {
                return Some(f.offset);
            }
        }
        None
    }

    fn get_elf_fn_info<'a>(object: &'a object::File, fn_info: &mut Vec<FunctionInfo>) {
        for sym in object.symbols() {
            if sym.kind() == object::SymbolKind::Text {
                fn_info.push(FunctionInfo {
                    name: String::from(sym.name().unwrap()),
                    offset: sym.address(),
                });
            }
        }
    }

    fn get_elf_var_info<'a>(object: &'a object::File, var_info: &mut Vec<VariableInfo>) {
        for sym in object.symbols() {
            if sym.kind() == object::SymbolKind::Data {
                var_info.push(VariableInfo {
                    name: String::from(sym.name().unwrap()),
                    offset: sym.address(),
                });
            }
        }
    }

    fn get_mmap_info_vec(
        pid: Pid,
        filename: &str,
        syscall_stack: &mut SyscallStack,
    ) -> (Vec<MapRange>, WaitStatus) {
        loop {
            let wait_options = WaitPidFlag::from_bits(
                WaitPidFlag::WCONTINUED.bits() | WaitPidFlag::WUNTRACED.bits(),
            );
            let status = waitpid(pid, wait_options).unwrap();

            if let Ok(m) = mem::get_mmap_info(pid, filename) {
                return (m, status);
            } else {
                catch_syscall(pid, syscall_stack);
            }
        }
    }

    pub fn exec_maps(&self) -> Result<Vec<&MapRange>, Box<dyn std::error::Error>> {
        let mut exec_maps = Vec::new();
        for m in &self.mmap_info_vec {
            if m.is_read() && !m.is_write() && m.is_exec() {
                exec_maps.push(m);
            }
        }
        if !exec_maps.is_empty() {
            Ok(exec_maps)
        } else {
            Err(Box::new(io::Error::new(
                io::ErrorKind::NotFound,
                "exec map not found",
            )))
        }
    }

    #[allow(unused)]
    pub fn data_maps(&self) -> Result<Vec<&MapRange>, Box<dyn std::error::Error>> {
        let mut data_maps = Vec::new();
        for m in &self.mmap_info_vec {
            if m.is_read() && m.is_write() && !m.is_exec() {
                data_maps.push(m);
            }
        }
        if !data_maps.is_empty() {
            Ok(data_maps)
        } else {
            Err(Box::new(io::Error::new(
                io::ErrorKind::NotFound,
                "exec map not found",
            )))
        }
    }

    pub fn rodata_maps(&self) -> Result<Vec<&MapRange>, Box<dyn std::error::Error>> {
        let mut rodata_maps = Vec::new();
        for m in &self.mmap_info_vec {
            if m.is_read() && !m.is_write() && !m.is_exec() {
                rodata_maps.push(m);
            }
        }
        if !rodata_maps.is_empty() {
            Ok(rodata_maps)
        } else {
            Err(Box::new(io::Error::new(
                io::ErrorKind::NotFound,
                "rodata map not found",
            )))
        }
    }

    // fn get_dwarf_fn_info<R: Reader<Offset = usize>>(
    //     dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    //     attrs: &mut AttrsIter<R>,
    // ) -> FunctionInfo {
    //     let mut offset = 0;
    //     let mut name = String::new();

    //     while let Some(attr) = attrs.next().unwrap() {
    //         match attr.name() {
    //             gimli::DW_AT_low_pc => {
    //                 offset = Self::get_dwarf_fn_offset(&attr.value()) as u64;
    //             }
    //             gimli::DW_AT_name => {
    //                 name = Self::get_dwarf_fn_name(dwarf, &attr.value());
    //             }
    //             _ => continue,
    //         }
    //     }

    //     FunctionInfo { name, offset }
    // }

    // fn get_dwarf_fn_offset<R: Reader<Offset = usize>>(val: &AttributeValue<R>) -> usize {
    //     match val {
    //         AttributeValue::Addr(offset) => offset.to_owned() as usize,
    //         _ => panic!("bad type!"),
    //     }
    // }

    // fn get_dwarf_fn_name<R: Reader<Offset = usize>>(
    //     dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    //     val: &AttributeValue<R>,
    // ) -> String {
    //     match val {
    //         AttributeValue::DebugStrRef(doffset) => {
    //             let debug_str = dwarf.debug_str;
    //             let s = debug_str
    //                 .get_str(doffset.to_owned())
    //                 .unwrap()
    //                 .to_string_lossy()
    //                 .into_owned();
    //             s
    //         }
    //         _ => panic!("bad type!"),
    //     }
    // }
}

#[allow(unused)]
pub fn dump_debug_info(filename: &str) {
    let file = fs::File::open(filename).unwrap();
    let mmap = unsafe { memmap::Mmap::map(&file).unwrap() };
    let object = object::File::parse(&*mmap).unwrap();
    let endian = if object.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };

    let dwarf_cow = get_dwarf_cow(&object).unwrap();
    let dwarf = get_dwarf(&dwarf_cow, endian);

    let mut iter = dwarf.units();
    while let Some(header) = iter.next().unwrap() {
        println!(
            "Unit at <.debug_info+0x{:x}>",
            header.offset().as_debug_info_offset().unwrap().0
        );
        let unit = dwarf.unit(header).unwrap();

        let mut depth = 0;
        let mut entries = unit.entries();
        while let Some((delta_depth, entry)) = entries.next_dfs().unwrap() {
            depth += delta_depth;
            println!("<{}><{:x}> {}", depth, entry.offset().0, entry.tag());

            let mut attrs = entry.attrs();
            while let Some(attr) = attrs.next().unwrap() {
                let value = attr.value();
                let value = match value {
                    AttributeValue::DebugStrRef(doffset) => {
                        let debug_str = dwarf.debug_str;
                        let s = debug_str
                            .get_str(doffset)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned();
                        s
                    }
                    AttributeValue::String(s) => s.to_string_lossy().into_owned(),
                    AttributeValue::Udata(ud) => format!("0x{:x}", ud),
                    AttributeValue::Flag(f) => format!("{f}"),
                    AttributeValue::FileIndex(i) => {
                        let debug_line = dwarf.debug_line;
                        let program = debug_line
                            .program(DebugLineOffset(0), 8, None, None)
                            .unwrap();
                        let (program, _sequence) = program.sequences().unwrap();
                        let file_names = program.header().file_names();
                        dwarf
                            .attr_string(&unit, file_names[(i - 1) as usize].path_name())
                            .unwrap()
                            .to_string_lossy()
                            .into_owned()
                            .to_string()
                    }
                    AttributeValue::UnitRef(uoffset) => {
                        // let entry = unit.entry(uoffset).unwrap();
                        format!("{:?}", uoffset)
                    }
                    AttributeValue::Data1(d) => format!("Data1(0x{:02x})", d),
                    AttributeValue::Data2(d) => format!("Data2(0x{:04x})", d),
                    AttributeValue::Data4(d) => format!("Data4(0x{:08x})", d),
                    AttributeValue::Data8(d) => format!("Data8(0x{:016x})", d),
                    AttributeValue::Addr(addr) => format!("Addr(0x{:016x})", addr),
                    AttributeValue::Encoding(ate) => ate.static_string().unwrap().to_string(),
                    AttributeValue::Exprloc(e) => {
                        let eval_result = e.evaluation(unit.encoding()).evaluate().unwrap();
                        match eval_result {
                            EvaluationResult::Complete => "Evaluation(Complete)".to_string(),
                            EvaluationResult::RequiresMemory {
                                address,
                                size,
                                space,
                                base_type,
                            } => format!(
                                "Evaluation(RequiresMemory) - address: 0x{:016x}, size: {:02x}, space: {:?}, base_type: {:?}",
                                address, size, space, base_type
                            ),
                            EvaluationResult::RequiresRegister {
                                register, base_type
                            } => format!("Evaluation(RequiresRegister) - register: {:?}, base_type: {:?}", register, base_type),
                            EvaluationResult::RequiresFrameBase => "Evaluation(RequiresFrameBase)".to_string(),
                            EvaluationResult::RequiresTls(tls) => format!("Evaluation(RequiresTls) - tls: {tls}"),
                            EvaluationResult::RequiresCallFrameCfa => "Evaluation(RequiresCallFrameCfa)".to_string(),
                            EvaluationResult::RequiresAtLocation(die_ref) => {
                                match die_ref {
                                    DieReference::UnitRef(uoffset) => format!("Evaluation(RequiresAtLocation) - die_reference: {:?}", uoffset),
                                    DieReference::DebugInfoRef(dioffset) => format!("Evaluation(RequiresAtLocation) - die_reference: {:?}", dioffset),
                                }
                            },
                            EvaluationResult::RequiresEntryValue(e) => format!("Evaluation(RequiresEntryValue) - expr: {:?}", e),
                            EvaluationResult::RequiresParameterRef(uoffset) => format!("Evaluation(RequiresParameterRef) - offset: {:?}", uoffset),
                            _ => "Exprloc".to_string(),
                        }
                    }
                    _ => format!("{:?}", value),
                };
                println!("   {}: {}", attr.name(), value);
            }
        }
    }
}

fn get_dwarf<'a>(
    dwarf_cow: &'a Dwarf<Cow<'a, [u8]>>,
    endian: gimli::RunTimeEndian,
) -> Dwarf<EndianSlice<'a, RunTimeEndian>> {
    let borrow_section: &dyn for<'bs> Fn(
        &'bs borrow::Cow<[u8]>,
    ) -> EndianSlice<'bs, RunTimeEndian> = &|section| EndianSlice::new(&*section, endian);

    dwarf_cow.borrow(&borrow_section)
}

fn get_dwarf_cow<'a>(object: &'a object::File) -> Result<Dwarf<Cow<'a, [u8]>>, gimli::Error> {
    let load_section = |id: gimli::SectionId| -> Result<borrow::Cow<[u8]>, gimli::Error> {
        match object.section_by_name(id.name()) {
            Some(ref section) => Ok(section
                .uncompressed_data()
                .unwrap_or(borrow::Cow::Borrowed(&[][..]))),
            None => Ok(borrow::Cow::Borrowed(&[][..])),
        }
    };

    Dwarf::load(&load_section)
}
