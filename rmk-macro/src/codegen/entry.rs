use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use rmk_config::SplitConnection;
use rmk_config::resolved::hardware::{BoardConfig, CommunicationConfig};
use rmk_config::resolved::{Behavior, Hardware, Host};
use syn::{ItemFn, ItemMod};

use super::override_helper::Overwritten;
use crate::codegen::chip::flash::expand_dfu_interface;
use crate::codegen::feature::{get_rmk_features, is_feature_enabled};

#[allow(clippy::too_many_arguments)]
pub(crate) fn expand_rmk_entry(
    hardware: &Hardware,
    host: &Host,
    behavior: &Behavior,
    item_mod: &ItemMod,
    devices: Vec<TokenStream2>,
    processors: Vec<TokenStream2>,
    registered_processors: Vec<TokenStream2>,
    watchdog_task: Option<TokenStream2>,
) -> TokenStream2 {
    // If there is a function with `#[Overwritten(entry)]`, override the entry
    if let Some((_, items)) = &item_mod.content {
        items
            .iter()
            .find_map(|item| {
                if let syn::Item::Fn(item_fn) = &item
                    && let Some(Ok(Overwritten::Entry)) =
                        super::override_helper::find_overwritten(item_fn)
                {
                    return Some(override_rmk_entry(item_fn));
                }
                None
            })
            .unwrap_or(rmk_entry_select(
                hardware,
                host,
                behavior,
                devices,
                processors,
                registered_processors,
                watchdog_task,
            ))
    } else {
        rmk_entry_select(
            hardware,
            host,
            behavior,
            devices,
            processors,
            registered_processors,
            watchdog_task,
        )
    }
}

fn override_rmk_entry(item_fn: &ItemFn) -> TokenStream2 {
    let content = &item_fn.block.stmts;
    quote! {
        #(#content)*
    }
}

pub(crate) fn rmk_entry_select(
    hardware: &Hardware,
    host: &Host,
    behavior: &Behavior,
    devices: Vec<TokenStream2>,
    processors: Vec<TokenStream2>,
    registered_processors: Vec<TokenStream2>,
    watchdog_task: Option<TokenStream2>,
) -> TokenStream2 {
    let auto_mouse_layer_enabled = !behavior.auto_mouse_layer.is_empty();
    let auto_mouse_layer_prelude = auto_mouse_layer_enabled.then(|| {
        quote! {
            let mut auto_mouse_layer = ::rmk::AutoMouseLayerRunner::new(&keymap);
        }
    });
    let devices_task = {
        let mut devs = devices.clone();
        devs.push(quote! {matrix});
        if hardware.storage.is_some() {
            devs.push(quote! {storage});
        }
        if auto_mouse_layer_enabled {
            devs.push(quote! {auto_mouse_layer});
        }
        quote! {
            ::rmk::run_all! (
                #(#devs),*
            )
        }
    };
    let processors_task = if processors.is_empty() {
        quote! {}
    } else {
        quote! {
            ::rmk::run_all! (
                #(#processors),*
            )
        }
    };

    let board = &hardware.board;
    let communication = &hardware.communication;
    // A BLE split central's transport takes one matrix region per peripheral.
    let split_peripheral_matrices = match board {
        BoardConfig::Split(split) if matches!(split.connection, SplitConnection::Ble) => {
            let peripheral_matrices = split.peripheral.iter().map(|p| {
                let rows = p.rows as u8;
                let cols = p.cols as u8;
                let row_offset = p.row_offset as u8;
                let col_offset = p.col_offset as u8;
                quote! {
                    ::rmk::split::PeripheralMatrixConfig {
                        rows: #rows,
                        cols: #cols,
                        row_offset: #row_offset,
                        col_offset: #col_offset,
                    }
                }
            });
            quote! { , [#(#peripheral_matrices),*] }
        }
        _ => quote! {},
    };
    // The DFU updater task only attaches to a USB transport. Split centrals
    // seed the transport's passthrough alt slots from the peripheral count.
    let communication_uses_usb = matches!(
        communication,
        CommunicationConfig::Usb(_) | CommunicationConfig::Both(_, _)
    );
    let (dfu_interface, num_peripherals) = if cfg!(feature = "_dfu") && communication_uses_usb {
        let num_peripherals = match board {
            BoardConfig::Split(split) => split.peripheral.len(),
            BoardConfig::UniBody(_) => 0,
        };
        (expand_dfu_interface(hardware.dfu.as_ref()), num_peripherals)
    } else {
        (quote! {}, 0)
    };
    let (transport_prelude, transport_tasks) = transport_setup(
        host,
        communication,
        split_peripheral_matrices,
        &dfu_interface,
        num_peripherals,
    );

    let entry = match board {
        BoardConfig::Split(split_config) => {
            let keyboard_task = quote! {
                keyboard.run(),
            };
            let mut tasks = vec![devices_task, keyboard_task];
            tasks.extend(registered_processors);
            tasks.extend(transport_tasks);
            if let Some(t) = &watchdog_task {
                tasks.push(t.clone());
            }
            match split_config.connection {
                SplitConnection::Ble => {
                    if !processors.is_empty() {
                        tasks.push(processors_task);
                    };
                    let joined = join_all_tasks(tasks);
                    quote! {
                        #transport_prelude
                        #auto_mouse_layer_prelude
                        #joined
                    }
                }
                SplitConnection::Serial => {
                    if !processors.is_empty() {
                        tasks.push(processors_task);
                    };
                    let central_serials = split_config
                        .central
                        .serial
                        .clone()
                        .expect("No serial defined for central");
                    split_config
                        .peripheral
                        .iter()
                        .enumerate()
                        .for_each(|(idx, p)| {
                            let row = p.rows as u8;
                            let col = p.cols as u8;
                            let row_offset = p.row_offset as u8;
                            let col_offset = p.col_offset as u8;
                            let uart_instance =
                                format_ident!(
                            "{}",
                            central_serials
                                .get(idx)
                                .expect("No or not enough serial defined for peripheral in central")
                                .instance
                                .to_lowercase()
                        );
                            let rmk_features = get_rmk_features();
                            let dfu_split_enabled = is_feature_enabled(&rmk_features, "dfu_split");
                            let policy = if dfu_split_enabled {
                                match p.update_policy {
                                    Some(rmk_config::UpdatePolicy::Force) => {
                                        quote! { ::rmk::split::central::UpdatePolicy::Force }
                                    }
                                    _ => quote! { ::rmk::split::central::UpdatePolicy::MatchHash },
                                }
                            } else {
                                quote! {}
                            };
                            tasks.push(quote! {
                                ::rmk::split::central::run_peripheral_manager(
                                    #idx,
                                    #uart_instance,
                                    ::rmk::split::PeripheralMatrixConfig {
                                        rows: #row,
                                        cols: #col,
                                        row_offset: #row_offset,
                                        col_offset: #col_offset,
                                    },
                                    #policy
                                )
                            });
                        });

                    let joined = join_all_tasks(tasks);
                    quote! {
                        #transport_prelude
                        #auto_mouse_layer_prelude
                        #joined
                    }
                }
            }
        }
        BoardConfig::UniBody(_) => rmk_entry_unibody(
            transport_prelude,
            auto_mouse_layer_prelude,
            transport_tasks,
            devices_task,
            processors_task,
            registered_processors,
            watchdog_task,
        ),
    };

    quote! {
        use ::rmk::core_traits::Runnable;
        #entry
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rmk_entry_unibody(
    transport_prelude: TokenStream2,
    auto_mouse_layer_prelude: Option<TokenStream2>,
    transport_tasks: Vec<TokenStream2>,
    devices_task: TokenStream2,
    processors_task: TokenStream2,
    registered_processors: Vec<TokenStream2>,
    watchdog_task: Option<TokenStream2>,
) -> TokenStream2 {
    let keyboard_task = quote! {
        keyboard.run()
    };

    let mut tasks = vec![devices_task, keyboard_task];
    if !processors_task.is_empty() {
        tasks.push(processors_task);
    }
    tasks.extend(registered_processors);
    tasks.extend(transport_tasks);
    if let Some(t) = watchdog_task {
        tasks.push(t);
    }
    let joined = join_all_tasks(tasks);
    quote! {
        #transport_prelude
        #auto_mouse_layer_prelude
        #joined
    }
}

/// Build (`let transport = ...;` prelude, transport run tasks) for the active
/// communication config.
fn transport_setup(
    host: &Host,
    communication: &CommunicationConfig,
    split_peripheral_matrices: TokenStream2,
    dfu_interface: &TokenStream2,
    num_peripherals: usize,
) -> (TokenStream2, Vec<TokenStream2>) {
    let wpm_prelude = quote! {
        let mut wpm_processor = ::rmk::processor::builtin::wpm::WpmProcessor::new();
    };
    let wpm_task = quote! { wpm_processor.run() };

    let host_active = host.vial_enabled || host.rynk_enabled;

    let with_host = if host_active {
        quote! { .with_host_service(&host_service) }
    } else {
        quote! {}
    };

    let dfu_split_enabled = is_feature_enabled(&get_rmk_features(), "dfu_split");
    let usb_prelude = if dfu_split_enabled && !dfu_interface.is_empty() {
        quote! {
            #dfu_interface
            let mut usb_transport = ::rmk::usb::UsbTransport::new(
                driver,
                rmk_config.device_config,
                #num_peripherals,
            )#with_host;
        }
    } else if !dfu_interface.is_empty() {
        quote! {
            #dfu_interface
            let mut usb_transport = ::rmk::usb::UsbTransport::new(driver, rmk_config.device_config)#with_host;
        }
    } else {
        quote! {
            let mut usb_transport = ::rmk::usb::UsbTransport::new(driver, rmk_config.device_config)#with_host;
        }
    };
    let ble_prelude = quote! {
        let mut ble_transport = ::rmk::ble::BleTransport::new(ble_controller, ble_addr, rmk_config #split_peripheral_matrices) #with_host;
    };
    let ble_task = quote! { ble_transport.run() };

    // The DFU updater task runs alongside the transport it feeds.
    let dfu_task = if dfu_interface.is_empty() {
        quote! {}
    } else {
        quote! { dfu_iface.run() }
    };

    match communication {
        CommunicationConfig::Usb(_) => {
            let prelude = quote! {
                #wpm_prelude
                #usb_prelude
            };
            let mut tasks = vec![quote! { usb_transport.run() }, wpm_task];
            if !dfu_task.is_empty() {
                tasks.push(dfu_task);
            }
            (prelude, tasks)
        }
        CommunicationConfig::Ble(_) => {
            let prelude = quote! {
                #wpm_prelude
                #ble_prelude
            };
            (prelude, vec![ble_task, wpm_task])
        }
        CommunicationConfig::Both(_, _) => {
            let prelude = quote! {
                #wpm_prelude
                #usb_prelude
                #ble_prelude
            };
            let mut tasks = vec![quote! { usb_transport.run() }, ble_task, wpm_task];
            if !dfu_task.is_empty() {
                tasks.insert(1, dfu_task);
            }
            (prelude, tasks)
        }
        CommunicationConfig::None => panic!("USB and BLE are both disabled"),
    }
}

pub(crate) fn expand_tasks(tasks: Vec<TokenStream2>) -> TokenStream2 {
    let mut current_joined = quote! {};
    tasks.iter().enumerate().for_each(|(id, task)| {
        if id == 0 {
            current_joined = quote! {#task};
        } else {
            current_joined = quote! {
                ::rmk::embassy_futures::join::join(#current_joined, #task)
            };
        }
    });
    current_joined
}

pub(crate) fn join_all_tasks(tasks: Vec<TokenStream2>) -> TokenStream2 {
    let joined = expand_tasks(tasks);
    quote! {
        #joined.await;
    }
}
