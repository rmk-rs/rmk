#![no_main]
#![no_std]

mod pointing_processor_controller;

use rmk::macros::rmk_keyboard;

#[rmk_keyboard]
mod keyboard {
    #[register_processor]
    fn pointing_processor_controller() -> crate::pointing_processor_controller::PointingProcessorController {
        crate::pointing_processor_controller::PointingProcessorController::new()
    }
}
