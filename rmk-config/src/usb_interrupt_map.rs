//! This file is auto-generated from https://github.com/embassy-rs/stm32-data-generated
//! DO NOT MODIFY

#![allow(dead_code)]
use once_cell::sync::Lazy;
use std::collections::HashMap;

use crate::communication::UsbInfo;

static USB_INFO: Lazy<HashMap<String, UsbInfo>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("esp32s3".to_string(), UsbInfo::new("GPIO19", "GPIO20", "USB_FS", "USB0"));
    // The ESP32-S31 high-speed OTG port has dedicated pins, so there is no D+/D- GPIO.
    m.insert("esp32s31".to_string(), UsbInfo::new("", "", "USB_HS", "USB_HS"));
    m.insert("nrf52840".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("nrf52820".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("nrf52833".to_string(), UsbInfo::new("", "", "USBD", "USBD"));
    m.insert("rp2040".to_string(), UsbInfo::new("", "", "USB", "USBCTRL_IRQ"));
    m.insert(
        "stm32h730vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g473qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0c1ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f207ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h562vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32u545ne".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l432kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f302ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32l072v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g474ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0c1me".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32h747ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4p5cg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f102c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l083rz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f750n8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b0ib".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32f102r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f302rd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f439ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595qj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32wb35ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f413mg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473pe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f413ch".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f427ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f215rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f207vf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b0rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l452ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l072rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l552ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32l152c6-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32l152vd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f479ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f479zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h742vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f217vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205rf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l422tb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f207ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g441kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f469ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l152vd-x".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h723ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f437vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h733vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l152re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u575vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f215ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f746bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f429vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a9nj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l475vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b3ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f479ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32u575oi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f102c4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f107rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b3zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32u083hc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f401cd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103vd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f723ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g441rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g471me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4p5re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496qe".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l062k8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f745vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g471ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l151cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l152qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f042c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f107vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32g0b1ke".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32l4s5qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413zh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32l476rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l053r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l552ze".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32f765bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l072kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32f070f6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32wb55vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f469vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h747ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f105r8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l162vc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f105rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f722zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l443vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l152rc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32l4p5zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h747xg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f777bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f427zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l083vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f411ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f078vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4p5ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l072vz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f730z8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f427ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l053c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32f042k4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h755ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f733ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f769bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f105vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f105vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h562ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32wb55rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f105v8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476mg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h562zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32g473me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l083v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32u073c8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f105rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f745ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1ke".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32f078rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l072rz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h743bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f733ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l083rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f103zf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32l443rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f303cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h757ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f479vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h742zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f107vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g484qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h745xg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h742ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l475rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32h745ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f723ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g491cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f437ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4q5ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f401ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g471re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l152uc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f042g6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l422rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g441mb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f779bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f107rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f429zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a5qj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f429ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g441vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h7a3ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l4p5ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f745ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162qd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u575ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f437zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4q5zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103rd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32l162vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4q5cg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0b1me".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f205vf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h742ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l152rd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f217ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575cg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0b1ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f469ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l422kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7a3lg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h742xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f437ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l152ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h745zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g471qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32u535ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l151rb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32l151zc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f429ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u599nj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h562ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f207vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l152c8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h730ib".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f469zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f217ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f446ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32l072kz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f469ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32h562ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32u545je".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f765ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f215vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f746ne".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b0vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f429ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l072vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32f102cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f439vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f769ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l152v8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f756bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l432kc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f302vd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f413rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473qb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l083vz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f207if".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h723vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f427vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l475ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f415og".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f103r8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32l162qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f479bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l052k8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32f072vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f401cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469ne".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l412r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7a3zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f723ic".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f767vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h753ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l073cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h563vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32g471mc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103c6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32g471vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u083rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32f072r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l152qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l082cz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f746zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l151r8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h7a3ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32g0b1kc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f429ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1ne".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32l152cb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h747bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u585zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u545ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l063c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l552zc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32wb55ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f765ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g0b0ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g473rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u585ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f722ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f048c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g473mb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f746ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f750v8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595rj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l152vb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f769ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f411cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l562ze".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32f070rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h750xb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h743vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f423zh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f417ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103r4".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32g473qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l462ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l562ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32g431m6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476jg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f765zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f423ch".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f373rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h750zb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431v6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g474cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f469ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1mc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32g4a1ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f769ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h725ae".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f302rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32l496ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f777zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h573zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32f373vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g473pc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l496zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u585ci".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h573ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32wb35cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h735vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h725ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f439bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f405rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l552cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32f373v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l452cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32u083kc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32u5a9vj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l151zd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f303zd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32l152vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103tb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32u535cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g484me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f407vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g484ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l162re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l412k8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f207zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l052c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l152c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f417ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l412tb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l052r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l152r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4r9vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h725ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h7a3rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l152rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32l433cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l496re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4p5qe".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0b1mb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32l4s7vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162vd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f415vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l100c6-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32u5a5vj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h7a3ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h725rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f479ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h742bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g484re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1mc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32l162ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l152rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l412t8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f407ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l433cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32g0b1vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32h725vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l412kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h563ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l486zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162rd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f415rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f429bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a5rj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f303ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h7a3vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l152vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f779ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32g0b1rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32g471qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h563zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f767zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151ze".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32wb55cg".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l073cz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f103t8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32l151vc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32l152v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4s5vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f417vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473pb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f373vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f407ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f446zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32g0c1rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f302r8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f302c6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f405vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r7vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f373r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f373rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h743zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h735rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l4s9vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h735ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f302vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h503kb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h743ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u599vj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g473vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f469bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h743xg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h503rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32f048g6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32wb55re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f405og".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473mc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f765vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u545re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f746ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f429be".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302k8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32g0c1kc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f756zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595vj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f103t4".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32f070c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l151r6-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32g431k6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h743ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f072rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g483ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1kb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f732ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l152qd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l052t8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f103v8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32u595zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32g0b1ne".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32l412rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l475re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f042f6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4r5ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32g491ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412cg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u599zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f723vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f401cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g484pe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g471rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4r9ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h753vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f072v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h563ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l4a6ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4a6zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h563rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l4r9zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u083mc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h745bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32u535rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l151uc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32g0b1cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32l433vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7a3ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32u599vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f723zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l082kz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h753zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l052r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4r9vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l152r6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h753ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4a6vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f746ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7a3ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32u535vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l433rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4r5vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l152cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l073vz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32wb55vg".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l052c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l152c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f732ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l152rb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32u595vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l476rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103zc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f429ne".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f423rh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f746ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l462re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f756vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l151c8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32l562re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32u595aj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g431rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103c4".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f439bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f769ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595zj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g474rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f373cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f302cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f469ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f722re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474mb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u599zj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l151v8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32g4a1re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l552rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32l452rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f765zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h503eb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f722ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u585ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431m8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l475vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f405zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r7zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l063r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h725re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l4s9ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h743vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f777ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4p5qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h573ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32g473cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h573ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h755bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4s9zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l100rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g0b0re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l100r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h725ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f769ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r7ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g4a1ke".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l100c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4s5zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f417zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f446vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f765ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f407ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l552qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32h750ib".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073hb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f446mc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474pc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h747bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f070cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4s5ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f411rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h563mi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l486vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103c8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32h725zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h7a3ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g483me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l073v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f429ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496ae".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f401rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7a3zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f303ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32l152zc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g483ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h725ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l151ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h563vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f767vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535ne".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l052k6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h753xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f479bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l073rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g491me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l476qe".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162ze".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l412c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f401vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g491ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476je".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303rd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f207ic".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f072c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7b3qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f103r6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32l151rd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u083cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f417ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r9zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l100r8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h745bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h563ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f401vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4s7ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f779ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a5aj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f303re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32f072cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g483re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u5a5zj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l4s7zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l162zd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f415zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l442kc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h563ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l4q5qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l486rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l412cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l082kb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4r9ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h757bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g471cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l052t6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l151vd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u5a9zj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f401rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g491re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f412rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303vd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f103cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f407zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f417ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535je".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f207vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l073vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7b3ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32wb55ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u073h8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h503cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f777vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h743ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431k8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h573vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32u073hc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32g474pb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h735ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g0b0ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h735zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32u545ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f205rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h573mi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32u585oi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302k6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h725ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l151rc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f411vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g473cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f407ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f765vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f412ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l100rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h743xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f446rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h750vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f750z8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g431c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h743ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f423mh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g431mb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l562me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32f756ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l462ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l562ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32f302c8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32g0c1cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32g431vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f302cc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32f423vh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474mc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h743zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f412ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f042f4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g474rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f373cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u585vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l475rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f048t6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l552vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert("stm32f373c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l452vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g4a1me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f302r6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32g4a1ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g431r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f722ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7a3vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32g484ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103t6".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32l151c6-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f767zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h563zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l4a6rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h563ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h7b3li".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l151vd-x".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f429bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l433rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l152cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g491ke".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f746vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g483pe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f405oe".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f732ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32wb55rg".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l073rz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h742bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f732re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f479ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f469be".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0b1cb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32f429ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b0zb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f746be".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l053c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f427ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f215zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f756ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f769bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302zd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32g473ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l562qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32f427zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h747xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h723zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h7b0ab".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32u585qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h747ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f207zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073mc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32u599bj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f446ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073m8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f217ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l552qc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32l162vd-x".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32l152r6-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32g474pe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f446me".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h755xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f765bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f411re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l053r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7a3qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f215re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u599ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h753bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151v8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f401rd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32l152ze".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32f102r4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l151vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l4p5ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535nc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g441cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f205zf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g491mc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l162zc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f479ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f401ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575og".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303rb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert("stm32g491vc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f207ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f768ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0b1ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert(
        "stm32u575vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l100rb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32l162rc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f723ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f429vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f779ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f042k6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4p5ae".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l062c8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l4p5ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f437vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4q5vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151qd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h742vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h7b3ri".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f479zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l152vc-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32l476re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b3ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f479ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32f042c4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f730r8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f427ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f745ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4p5rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f730i8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l496qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f102r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h755zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103vf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f722rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073kb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32f102c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h747ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l083cz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l452re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32l072cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h562vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32l552re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32h747zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a5qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l476zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413rh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f722ic".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g474ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f103rf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32f745ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4p5vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f427vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f777ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f733ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073kc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32f042g4".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f730v8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32g0c1ce".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_UCPD1_2"),
    );
    m.insert("stm32g474me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f769ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f439vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073k8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f413vh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert("stm32f102rb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f765ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l452ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h562rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32l552ve".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32f469ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l552me".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_FS"));
    m.insert(
        "stm32h562ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f722vc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413cg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413mh".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h745zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f103zd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN1_RX0"),
    );
    m.insert(
        "stm32l486qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4q5rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32f042t6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f429ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g483qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f745zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l151qe".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32u575rg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l486jg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f437ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32g491kc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32h742xg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575ci".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h757zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u5a9bj".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h742ii".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7a3li".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32l476ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l476me".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32u535re".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h757ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7b3vi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h723ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f429ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h7a3ni".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l151rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l151r8".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f401vd".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h733zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32h757xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f767bg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f437zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f205zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4a6qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u575zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h745ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f429zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f746ng".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f215ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l4r5qg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l422cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert("stm32g471ce".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l151vb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f469ae".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f401re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f303vb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32h742ag".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h745xi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l152zd".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32g491rc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32l151c6".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32l152r8-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32h742zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f217zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f479vg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u595qi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert(
        "stm32f469ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f207ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32wb55vy".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert("stm32wb55cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m.insert(
        "stm32f429ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f413zg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32l151cb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert(
        "stm32f205re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f207zf".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073mb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h743bi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32u073rc".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32h562zi".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32l151vb-a".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP"),
    );
    m.insert("stm32f078cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32f411ve".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32h730zb".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l072cz".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32l496wg".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f469ig".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f778ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f302ze".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_LP_CAN_RX0"),
    );
    m.insert(
        "stm32u073r8".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert(
        "stm32f217ie".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert(
        "stm32f446re".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_FS", "OTG_FS"),
    );
    m.insert("stm32l443cc".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h730ab".to_string(),
        UsbInfo::new("PA11", "PA12", "USB_OTG_HS", "OTG_HS"),
    );
    m.insert("stm32l083cb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB"));
    m.insert(
        "stm32h562ai".to_string(),
        UsbInfo::new("PA11", "PA12", "USB", "USB_DRD_FS"),
    );
    m.insert("stm32g474qb".to_string(), UsbInfo::new("PA11", "PA12", "USB", "USB_LP"));
    m
});

pub fn get_usb_info(chip: &str) -> Option<UsbInfo> {
    USB_INFO.get(chip).cloned()
}
