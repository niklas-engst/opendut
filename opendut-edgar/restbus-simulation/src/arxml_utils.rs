/*
    HELPER METHODS.
    Some are oriented on https://github.com/DanielT/autosar-data/blob/main/autosar-data/examples/businfo/main.rs.
*/
use crate::arxml_structs::*;
use crate::restbus_structs::*;
use crate::restbus_utils::*;

use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::vec;

use anyhow::{anyhow, bail, Result};

use autosar_data::{CharacterData, Element, ElementName, EnumItem};

use nix::libc::timeval;

use tracing::warn;

/*
    Converts a CharacterData type from the autosar_data library
    Directly taken from https://github.com/DanielT/autosar-data/blob/main/autosar-data/examples/businfo/main.rs.
*/
pub fn decode_integer(cdata: &CharacterData) -> Option<u64> {
    if let CharacterData::String(text) = cdata {
        if text == "0" {
            Some(0)
        } else if text.starts_with("0x") || text.starts_with("0X") {
            Some(u64::from_str_radix(&text[2..], 16).ok()?)
        } else if text.starts_with("0b") || text.starts_with("0B") {
            Some(u64::from_str_radix(&text[2..], 2).ok()?)
        } else if let Some(stripped_octal) = text.strip_prefix('0') {
            Some(u64::from_str_radix(stripped_octal, 8).ok()?)
        } else if text.to_ascii_lowercase() == "false" {
            Some(0)
        } else if text.to_ascii_lowercase() == "true" {
            Some(1)
        } else {
            Some(text.parse().ok()?)
        }
    } else {
        None
    }
}

/*
    Processes time-related element data (intended from a ISignalIPdu element) and returns a self-defined TimeRange struct.
*/
pub fn get_time_range(base: &Element) -> Option<TimeRange> {
    let value = base
        .get_sub_element(ElementName::Value)
        .and_then(|elem| elem.character_data())
        .and_then(|cdata| cdata.float_value())?;

    let tolerance = if let Some(absolute_tolerance) = base
        .get_sub_element(ElementName::AbsoluteTolerance)
        .and_then(|elem| elem.get_sub_element(ElementName::Absolute))
        .and_then(|elem| elem.character_data())
        .and_then(|cdata| cdata.float_value())
    {
        Some(TimeRangeTolerance::Absolute(absolute_tolerance))
    } else {
        base.get_sub_element(ElementName::RelativeTolerance)
            .and_then(|elem| elem.get_sub_element(ElementName::Relative))
            .and_then(|elem| elem.character_data())
            .and_then(|cdata| decode_integer(&cdata))
            .map(TimeRangeTolerance::Relative)
    };

    Some(TimeRange { tolerance, value })
}

/*
    Gets a sub-element and tries to extract time-related information.
*/
pub fn get_sub_element_and_time_range(base: &Element, sub_elem_name: ElementName, value: &mut f64, tolerance: &mut Option<TimeRangeTolerance>) {
    if let Some(time_range) = base 
        .get_sub_element(sub_elem_name)
        .and_then(|elem| get_time_range(&elem)) 
    {
        *value = time_range.value;
        *tolerance = time_range.tolerance;
    }
}

/*
    Gets a required subsubelement from the element. This needs to succeed. 
*/
pub fn get_required_sub_subelement(element: &Element, subelement_name: ElementName, sub_subelement_name: ElementName) -> Result<Element> {
    element.get_sub_element(subelement_name)
        .and_then(|elem| elem.get_sub_element(sub_subelement_name))
        .ok_or_else(|| anyhow!("Sub-sub-element of {subelement_name}->{sub_subelement_name} does not exist"))
}

/*
    Tries to get a subelement and convert it's value to u64.
*/
pub fn get_subelement_int_value(element: &Element, subelement_name: ElementName) -> Option<u64> {
    element 
        .get_sub_element(subelement_name)
        .and_then(|elem| elem.character_data())
        .and_then(|cdata| decode_integer(&cdata))
} 

/*
    Gets the u64 value for a element. This has to succeed.
*/
pub fn get_required_int_value(element: &Element, subelement_name: ElementName) -> Result<u64> {
    get_subelement_int_value(element, subelement_name)
        .ok_or_else(|| anyhow!("Error getting required integer value of {subelement_name}"))
}

/*
    Gets the u64 value for a element. This is optional. So, if the subelement does not exist, then 0 is returned.
*/
pub fn get_optional_int_value(element: &Element, subelement_name: ElementName) -> u64 {
    get_subelement_int_value(element, subelement_name).unwrap_or_default()
}

/*
    Resolves a reference and returns the target Element. This has to succeed.
*/
pub fn get_required_reference(element: &Element, subelement_name: ElementName) -> Result<Element> {
    if let Some(subelement) = element.get_sub_element(subelement_name){
        match subelement.get_reference_target() {
            Ok(reference) => return Ok(reference),
            Err(err) => {
                warn!("[-] Warning: Constant ref error: {}. Will try modification of target name and reference again.", err);
                match subelement.character_data() {
                    Some(val) => {
                        let new_dest = "/Constants/".to_string() + val.to_string().as_str();
                        match subelement.set_character_data(CharacterData::String(new_dest)) {
                            Ok(()) => {}
                            Err(err) => warn!("[-] Warning: Error setting new dest: {}", err)
                        }
                        match subelement.get_reference_target() {
                            Ok(reference) => return Ok(reference),
                            Err(err) => warn!("[-] Warning: Constant ref retry error: {}.", err),
                        }
                    }
                    _ => warn!("[-] Warning: No success in retry because Element CharacterData is not set."),
                }
            }
        }
    }
    
    bail!("Error getting required reference for {}", subelement_name)
}

/*
    Tries to get a subelement and return it's String value. 
*/
pub fn get_subelement_string_value(element: &Element, subelement_name: ElementName) -> Option<String> {
    element 
        .get_sub_element(subelement_name)
        .and_then(|elem| elem.character_data())
        .map(|cdata| cdata.to_string())
}

/*
    Gets the String value for a element. This is optional. So, if the subelement does not exist, then "" is returned.
*/
pub fn get_optional_string(element: &Element, subelement_name: ElementName) -> String {
    get_subelement_string_value(element, subelement_name).unwrap_or_default()
}

/*
    Gets the String value of a subsubelement. In case the subelement or subsubelement do not exist, then "" is returned.
*/
pub fn get_subelement_optional_string(element: &Element, subelement_name: ElementName, sub_subelement_name: ElementName) -> String {
    element.get_sub_element(subelement_name)
        .and_then(|elem| elem.get_sub_element(sub_subelement_name))
        .and_then(|elem| elem.character_data())
        .map(|cdata| cdata.to_string()).unwrap_or_default()
}

/*
    Retrieves the ECU name by resolving multiple references.
*/
pub fn ecu_of_frame_port(frame_port: &Element) -> Option<String> {
    let ecu_comm_port_instance = frame_port.parent().ok()??;
    let comm_connector = ecu_comm_port_instance.parent().ok()??;
    let connectors = comm_connector.parent().ok()??;
    let ecu_instance = connectors.parent().ok()??;
    ecu_instance.item_name()
}

/*
    Helper method comparing a given String with a byte order indicator. 
    Returns true for Big Endian, false for Little Endian
*/
pub fn get_byte_order(byte_order: &String) -> bool {
    if byte_order.eq("MOST-SIGNIFICANT-BYTE-LAST") {
        return false;
    }
    true
}

fn process_isignal_init_value(isignal: &ISignal, bits: &mut [bool]) -> Result<()>{
    let mut tmp_bit_array: Vec<bool> = Vec::new();
    let init_values = &isignal.init_values;
    let isignal_byte_order = isignal.byte_order;
    let isignal_length: usize = isignal.length.try_into()?;
    let isignal_start: usize = isignal.start_pos.try_into()?;

    match init_values {
        InitValues::Single(value) => {
            let mut n = *value;

            while n != 0 {
                tmp_bit_array.push(n & 1 != 0);
                n >>= 1;
            }

            while tmp_bit_array.len() < isignal_length {
                tmp_bit_array.push(false);
            }
    
            if isignal_byte_order {
                tmp_bit_array.reverse();
            }
        }
        InitValues::Array(values) => {
            if isignal_length % 8 != 0 {
                bail!("ISignal length for array is not divisible by 8. Length is {}", isignal_length)
            }

            for isignal_value in values {
                let byte_len: usize = 8;
                let mut n = *isignal_value;
                let mut tmp_tmp_bit_array: Vec<bool> = Vec::new();

                while n != 0 {
                    tmp_tmp_bit_array.push(n & 1 != 0);
                    n >>= 1;
                }

                while tmp_tmp_bit_array.len() < byte_len {
                    tmp_tmp_bit_array.push(false);
                }
                    
                tmp_tmp_bit_array.reverse();

                tmp_bit_array.extend(tmp_tmp_bit_array);
            }
        }
        _ => return Ok(())
    }

    if tmp_bit_array.len() != <u64 as TryInto<usize>>::try_into(isignal.length)? {
        bail!("Miscalculation for tmp_bit_array")
    }

    let mut index: usize = 0;

    while index < isignal_length {
        bits[isignal_start + index] = tmp_bit_array[index];
        index += 1;
    }

    Ok(())
} 

/* 
    Extracts the initial values for a PDU by processing contained ISignal and ISignalGroup elements related to that PDU.
    See how endianess affects PDU in 6.2.2 https://www.autosar.org/fileadmin/standards/R22-11/CP/AUTOSAR_TPS_SystemTemplate.pdf
    Currenlty assumes Little Endian byte ordering and has support for signals that are Little Endian or Big Endian.
    Bit positions in undefined ranges are set to unused_bit_pattern.
*/
pub fn extract_init_values(unused_bit_pattern: bool, ungrouped_signals: &Vec<ISignal>, grouped_signals: &Vec<ISignalGroup>, length: u64, byte_order: &bool) -> Result<Vec<u8>> {
    let dlc: usize = length.try_into()?;

    let mut bits = vec![unused_bit_pattern; dlc * 8]; // Using unusued_bit_pattern for undefined bits 

    for isignal in ungrouped_signals {
        process_isignal_init_value(isignal, &mut bits)?;
    }
    
    for isignal_group in grouped_signals {
        for isignal in &isignal_group.isignals {
            process_isignal_init_value(isignal, &mut bits)?;
        }
    }

    let mut init_values: Vec<u8> = Vec::new();
    let mut current_byte: u8 = 0;
    let mut bit_count = 0;
        
    for bit in bits {
        current_byte <<= 1;
        if bit {
            current_byte |= 1;
        }
        bit_count += 1;
   
        if bit_count == 8 {
            init_values.push(current_byte);
            current_byte = 0;
            bit_count = 0;
        }
    }
    if bit_count > 0 {
        current_byte <<= 8 - bit_count;
        init_values.push(current_byte);
    }

    if !byte_order {
        for init_value in init_values.iter_mut() {
            *init_value = init_value.reverse_bits(); // reverse bits of each byte
        }
    }

    if init_values.len() != dlc {
        bail!("Error creating byte array")
    }

    Ok(init_values)
}

/*
    Extracts the bit value used for unused bits by the PDU and returns a bool representation.
*/
pub fn get_unused_bit_pattern(pdu: &Element) -> bool {
    // even though it needs to exist at least for ISignalIPdus, we keep it as optional, since at least one encounter shows that it might be missing.
    // then use 0 as default value
    let unused_bit_pattern_int = get_optional_int_value(pdu, ElementName::UnusedBitPattern); 
    
    // supports values > 1. Just look at least significant bit
    (unused_bit_pattern_int & 1) != 0
}

/*
    Processes the Autosar FramePortRefs elements inside a CanFrameTriggering to find out the ECUs (names) that send and receive this underlying CAN frame.
*/
pub fn process_frame_ports(can_frame_triggering: &Element, can_frame_triggering_name: &String, rx_ecus: &mut Vec<String>, tx_ecus: &mut Vec<String>) -> Result<()> {
    if let Some(frame_ports) = can_frame_triggering.get_sub_element(ElementName::FramePortRefs) {
        let frame_ports: Vec<Element> = frame_ports.sub_elements()
            .filter(|se| se.element_name() == ElementName::FramePortRef)
            .filter_map(|fpr| fpr.get_reference_target().ok())
            .collect();

        for frame_port in frame_ports {
            if let Some(ecu_name) = ecu_of_frame_port(&frame_port) {
                if let Some(CharacterData::Enum(direction)) = frame_port
                    .get_sub_element(ElementName::CommunicationDirection)
                    .and_then(|elem| elem.character_data())
                {
                    match direction {
                        EnumItem::In => rx_ecus.push(ecu_name), 
                        EnumItem::Out => tx_ecus.push(ecu_name), 
                        _ => bail!("Invalid direction ID encountered in FramePort. Skipping CanFrameTriggering {}", can_frame_triggering_name)
                    }
                } else {
                    bail!("No CommunicationDirection encountered in FramePort. Skipping CanFrameTriggering {}", can_frame_triggering_name)
                }
            } else {
                bail!("Could not extract ECUName in FramePort. Skipping CanFrameTriggering {}", can_frame_triggering_name) ;
            }
        }
    }/* else {
        return Err(format!("FramePortRefs in CanFrameTriggering not found. Skipping CanFrameTriggering {}", can_frame_triggering_name));
    }*/

    Ok(())
}

/*
    Processes the Autosar InitValue element of an ISignal. Extracts one or more of them an put them into passed init_values argument.
*/
pub fn process_init_value(init_value_elem: &mut Element, init_values: &mut InitValues, signal_name: &String) -> Result<()> {
    let mut subelement = init_value_elem.get_sub_element_at(0)
        .ok_or_else(|| anyhow!("Failed to obtain subelement of init_value_elem"))?;
    
    if subelement.element_name().eq(&ElementName::ConstantReference) {
        let constant = get_required_reference(
            &subelement,
            ElementName::ConstantRef)?;

        *init_value_elem = constant.get_sub_element(ElementName::ValueSpec)
            .ok_or_else(|| anyhow!("Failed to obtain subelement of constant"))?;
    
        subelement = init_value_elem.get_sub_element_at(0)
            .ok_or_else(|| anyhow!("Failed to obtain subelement of init_value_elem"))?;
    }

    let init_value_type = match subelement.element_name() {
        ElementName::NumericalValueSpecification => 0,
        ElementName::ArrayValueSpecification => 1,
        _ => bail!("Unrecognized subelement {} for init-value", subelement.element_name())
    };

    if init_value_type == 0 {
        let num_val = init_value_elem.get_sub_element(ElementName::NumericalValueSpecification)
            .ok_or_else(|| anyhow!("InitValue element does not have NumercialValueSpecification for signal {}", signal_name))?;
        let init_value = get_required_int_value(&num_val, ElementName::Value)?;
        *init_values = InitValues::Single(init_value);


    } else {
        let mut init_value_array: Vec<u64> = Vec::new();
        let num_val_elements = get_required_sub_subelement(init_value_elem, 
            ElementName::ArrayValueSpecification, 
            ElementName::Elements);

        for num_val_elem in num_val_elements?.sub_elements() {
            init_value_array.push(get_required_int_value(&num_val_elem, ElementName::Value)?);
        }
        
        *init_values = InitValues::Array(init_value_array);
    }

    Ok(())
}

/*
    -Processes an ISignalGroup element and extracts important data.
    -Removes signals defined in ISignalGroup from signals HashMap (passed argument).
    -Pushes the resulting self-defined ISignalGroup structure containing important data into the grouped_signals argument.
*/
pub fn process_signal_group(signal_group: &Element, 
    signals: &mut HashMap<String, (String, String, u64, u64, InitValues)>, 
    grouped_signals: &mut Vec<ISignalGroup>) -> Result<()> 
    {
    let group_name = signal_group.item_name()
            .ok_or_else(|| Error::GetItemName{item: "ISignalGroupRef"})?;
    
    let mut signal_group_signals: Vec<ISignal> = Vec::new();

    let isignal_refs = signal_group.get_sub_element(ElementName::ISignalRefs)
        .ok_or_else(|| anyhow!("Element has no sub-element"))?;

    // Removing ok and needed?
    for isignal_ref in isignal_refs.sub_elements()
        .filter(|elem| elem.element_name() == ElementName::ISignalRef) {
        if let Some(CharacterData::String(path)) = isignal_ref.character_data() {
            if let Some(siginfo) = signals.get(&path) {
                let siginfo_tmp = siginfo.clone();
                let isginal_tmp: ISignal = ISignal {
                    name: siginfo_tmp.0,
                    byte_order: get_byte_order(&siginfo_tmp.1),
                    start_pos: siginfo_tmp.2,
                    length: siginfo_tmp.3,
                    init_values: siginfo_tmp.4
                };

                signal_group_signals.push(isginal_tmp);
                signals.remove(&path);
            }
        }
    }

    signal_group_signals.sort_by(|a, b| a.start_pos.cmp(&b.start_pos));

    let mut data_transformations: Vec<String> = Vec::new();

    if let Some(com_transformations) = signal_group
        .get_sub_element(ElementName::ComBasedSignalGroupTransformations) 
    {
        for elem in com_transformations.sub_elements() {
            let data_transformation = get_required_reference(&elem,
                ElementName::DataTransformationRef)?;
            
            let data_transformation_name = data_transformation.item_name()
                .ok_or_else(|| Error::GetItemName{item: "DataTransformation"})?;
            data_transformations.push(data_transformation_name);
        }
    }

    let mut props_vector: Vec<E2EDataTransformationProps> = Vec::new();

    if let Some(transformation_props) = signal_group.get_sub_element(ElementName::TransformationISignalPropss) {
        for e2exf_props in transformation_props
            .sub_elements()
            .filter(|elem| elem.element_name() == ElementName::EndToEndTransformationISignalProps)
        {
            if let Some(e2exf_props_cond) = e2exf_props
                .get_sub_element(ElementName::EndToEndTransformationISignalPropsVariants)
                .and_then(|elem| elem.get_sub_element(ElementName::EndToEndTransformationISignalPropsConditional))
            {
                let transformer_reference = get_required_reference(&e2exf_props_cond, 
                    ElementName::TransformerRef)?;
                
                let transformer_name = transformer_reference.item_name()
                    .ok_or_else(|| Error::GetItemName{item: "TransformerName"})?;

                let data_ids = e2exf_props_cond
                    .get_sub_element(ElementName::DataIds).ok_or_else(|| anyhow!("Element has no sub-element"))?;

                let data_id = get_required_int_value(&data_ids,
                    ElementName::DataId)?;

                // allow optional for now
                //let data_length = get_required_int_value(&e2exf_props_cond,
                let data_length = get_optional_int_value(&e2exf_props_cond,
                    ElementName::DataLength);
                
                
                let props_struct: E2EDataTransformationProps = E2EDataTransformationProps {
                    transformer_name,
                    data_id,
                    data_length 
                };

                props_vector.push(props_struct);
            }
        }
    }

    let isignal_group_struct: ISignalGroup = ISignalGroup {
        name: group_name,
        isignals: signal_group_signals,
        data_transformations,
        transformation_props: props_vector 
    };

    grouped_signals.push(isignal_group_struct);

    Ok(())
}

/*
    1. Extract data from CanFrameTriggering structure that is later needed by restbus-simulation. 
    2. Create TimedCanFrame sructure out of data and put the structure into timed_can_frames vector. 
    Note: Should normally only add one TimedCanFrame but multiple may be added in case multiple PDU Mappings exist for a Can frame.
*/
pub fn get_timed_can_frame(can_frame_triggering: &CanFrameTriggering, timed_can_frames: &mut Vec<TimedCanFrame>) -> Result<()> {
    let can_id: u32 = can_frame_triggering.can_id as u32;
    let len: u8 = can_frame_triggering.frame_length as u8;
    let addressing_mode: bool = can_frame_triggering.can_29_bit_addressing;
    let frame_tx_behavior: bool = can_frame_triggering.frame_tx_behavior;
    for pdu_mapping in &can_frame_triggering.pdu_mappings {
        let mut count: u32 = 0;
        let mut ival1_tv_sec: u64 = 0;
        let mut ival1_tv_usec: u64 = 0;
        let mut ival2_tv_sec: u64 = 0;
        let mut ival2_tv_usec: u64 = 0;
        let init_values: Vec<u8>;
        match &pdu_mapping.pdu {
            Pdu::ISignalIPdu(pdu) => {
                count = pdu.number_of_repetitions as u32;
                
                if pdu.repetition_period_value != 0.0 {
                    ival1_tv_sec = pdu.repetition_period_value.trunc() as u64;
                    let fraction: f64 = pdu.repetition_period_value % 1.0;
                    ival1_tv_usec = (fraction * 1_000_000.0).trunc() as u64;
                }

                if pdu.cyclic_timing_period_value != 0.0 {
                    ival2_tv_sec = pdu.cyclic_timing_period_value.trunc() as u64;
                    let fraction: f64 = pdu.cyclic_timing_period_value % 1.0;
                    ival2_tv_usec = (fraction * 1_000_000.0).trunc() as u64;
                }

                init_values = extract_init_values(pdu.unused_bit_pattern,
                        &pdu.ungrouped_signals,
                        &pdu.grouped_signals,
                        pdu_mapping.length,
                        &pdu_mapping.byte_order)?;
            }
            Pdu::NmPdu(pdu) => {
                ival2_tv_usec = 100000; // every 100 ms
                init_values = extract_init_values(pdu.unused_bit_pattern,
                        &pdu.ungrouped_signals,
                        &pdu.grouped_signals,
                        pdu_mapping.length,
                        &pdu_mapping.byte_order)?;
            }
        }

        let ival1 = timeval { tv_sec: ival1_tv_sec as TimevalNum, tv_usec: ival1_tv_usec as TimevalNum};
        let ival2 = timeval { tv_sec: ival2_tv_sec as TimevalNum, tv_usec: ival2_tv_usec as TimevalNum};

        let ivals: Vec<timeval> = vec![ival1, ival2];

        timed_can_frames.push(create_time_can_frame_structure(count, &ivals, can_id, len, addressing_mode, frame_tx_behavior, &init_values));
    }
    Ok(())
}

/*
    1. Find CanFrameTriggering structure based on CAN id.
    2. Put its important data as TimedCanFrame structure into timed_can_frames vector. 
*/
pub fn get_timed_can_frame_from_id(can_clusters: &HashMap<String, CanCluster>, bus_name: String, can_id: u64) -> Result<Vec<TimedCanFrame>> {
    let mut timed_can_frames: Vec<TimedCanFrame> = Vec::new();

    if let Some(can_cluster) = can_clusters.get(&bus_name) {
        if let Some(can_frame_triggering) = can_cluster.can_frame_triggerings.get(&can_id) {
            get_timed_can_frame(can_frame_triggering, &mut timed_can_frames)?;
        }
    }

    Ok(timed_can_frames)
}

/*
    1. Iterate over all CanFrameTriggerings belonging to a CanCluster structure. 
    2. Put all CanFrameTriggering important data as TimedCanFrame structures into timed_can_frames vector. 
*/
pub fn get_timed_can_frames_from_bus(can_clusters: &HashMap<String, CanCluster>, bus_name: String) -> Result<Vec<TimedCanFrame>> {
    let mut timed_can_frames: Vec<TimedCanFrame> = Vec::new();

    if let Some(can_cluster) = can_clusters.get(&bus_name) {
        for can_frame_triggering in can_cluster.can_frame_triggerings.values() {
            get_timed_can_frame(can_frame_triggering, &mut timed_can_frames)?
        }
    }

    Ok(timed_can_frames)
}

pub fn load_serialized_data(file_name: &String) -> Result<HashMap<String, CanCluster>> {
    let mut file = File::open(file_name.to_owned() + ".ser")?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
   
    let deserialized: HashMap<String, CanCluster> = serde_json::from_str(&contents)?;

    Ok(deserialized)
}

pub fn store_serialized_data(file_name: &String, can_clusters: &HashMap<String, CanCluster>) -> Result<()> {
    let serialized = serde_json::to_string(can_clusters)?;

    let mut file = File::create(file_name.to_owned() + ".ser")?;
    file.write_all(serialized.as_bytes())?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error<'a> {
    #[error("Failed to get required item name of '{item}'")]
    GetItemName { item: &'a str },
}