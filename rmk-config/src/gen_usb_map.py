import os
import json

path = "<your stm32-data-generated path>/data/chips"
path = "/Users/haobogu/Projects/keyboard/stm32-data-generated/data/chips"
chips = os.listdir(path)

s = set()
results = {}
for chip in chips:
    with open(os.path.join(path, chip), "r") as f:
        data = json.load(f)
        p = data["cores"][0]["peripherals"]
        interrupts = data["cores"][0]["interrupts"]
        flag = False
        has_usb = False
        for item in p:
            if "USB" in item["name"] and "USBRAM" not in item["name"]:
                has_usb = True
                if "interrupts" not in item:
                    continue
                for p in item["pins"]:
                    if p["signal"] == "DM":
                        dm = p["pin"]
                        # if p["pin"] != "PA11":
                        #     print(chip, "PIN", p)
                    if p["signal"] == "DP":
                        dp = p["pin"]
                        # if p["pin"] != "PA12":
                        #     print(chip, "PIN", p)
                peripheral = item["name"]
                for i in item["interrupts"]:
                    if i["signal"] == "GLOBAL":
                        s.add(f"{item['name']}____{i['interrupt']}")
                        interrupt = i["interrupt"]
                        flag = True
                        break
                    elif i["signal"] == "LP":
                        s.add(f"{item['name']}____{i['interrupt']}")
                        interrupt = i["interrupt"]
                        flag = True
                        break
                print(chip, peripheral, interrupt)
                chip_name = chip.replace(".json", "").lower()
                if chip_name in results and peripheral == "USB_OTG_HS" and results[chip_name]['peripheral'] == "USB_OTG_FS":
                    # Some chips have both `USB_OTG_HS` and `USB_OTG_FS`, we use `USB_OTG_FS` for now 
                    continue
                results[chip_name] = {
                    "peripheral": peripheral,
                    "interrupt": interrupt,
                    "dm": dm,
                    "dp": dp,
                }
lines = []
for k, v in results.items():
    line = f"    m.insert(\"{k}\".to_string(), UsbInfo::new(\"{v['dm']}\", \"{v['dp']}\", \"{v['peripheral']}\", \"{v['interrupt']}\"));"
    lines.append(line)

content = "\n".join(lines)

generated_file = """//! This file is auto-generated from https://github.com/embassy-rs/stm32-data-generated
//! DO NOT MODIFY

#![allow(dead_code)]
use std::collections::HashMap;
use once_cell::sync::Lazy;

use crate::UsbInfo;

static USB_INFO: Lazy<HashMap<String, UsbInfo>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("esp32s31".to_string(), UsbInfo::new("", "", "USB_HS", "USB_OTG_HS"));
    m.insert("nrf52840".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("nrf52820".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("nrf52833".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("rp2040".to_string(), UsbInfo::new("", "", "USB", "USBCTRL_IRQ"));
""" + content + """
    m
});

#[derive(Clone, Debug, Default)]
pub(crate) struct UsbInfo {
    pub(crate) dm: String,
    pub(crate) dp: String,
    pub(crate) peripheral_name: String,
    pub(crate) interrupt_name: String,
}

impl UsbInfo {
    pub(crate) fn new(dm: &str, dp: &str, p: &str, i: &str) -> Self {
        UsbInfo {
            dm: dm.to_string(),
            dp: dp.to_string(),
            peripheral_name: p.to_string(),
            interrupt_name: i.to_string(),
        }
    }

    pub(crate) fn new_default(chip: &ChipModel) -> Self {
        match chip.series {
            ChipSeries::Stm32 => UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "USB_FS"),
            ChipSeries::Nrf52 => UsbInfo::new("", "", "USBD", "USBD"),
            ChipSeries::Rp2040 => UsbInfo::new("", "", "USB", "USBCTRL_IRQ"),
            _ => UsbInfo::new(
                "default_dm",
                "default_dp",
                "default_usb_peripheral",
                "default_usb_interrupt",
            ),
        }
    }
}


pub fn get_usb_info(chip: &str) -> Option<UsbInfo> {
    USB_INFO.get(chip).cloned()
}
"""


with open("rmk-macro/src/usb_interrupt_map.rs", "w") as f:
    f.write(generated_file)
