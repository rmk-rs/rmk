use std::collections::HashMap;

use once_cell::sync::Lazy;

pub static KEYCODE_ALIAS: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();

    macro_rules! add_alias {
        ($keycode:tt) => {
            m.insert(paste::paste!{ stringify!([<$keycode:lower>]) }, $keycode);
        };
        ($keycode:tt = $( $alias:expr),*) => {
            add_alias!($keycode);
            $(
                m.insert($alias, $keycode);
            )*
        };
    }

    add_alias!("No");
    add_alias!("A");
    add_alias!("B");
    add_alias!("C");
    add_alias!("D");
    add_alias!("E");
    add_alias!("F");
    add_alias!("G");
    add_alias!("H");
    add_alias!("I");
    add_alias!("J");
    add_alias!("K");
    add_alias!("L");
    add_alias!("M");
    add_alias!("N");
    add_alias!("O");
    add_alias!("P");
    add_alias!("Q");
    add_alias!("R");
    add_alias!("S");
    add_alias!("T");
    add_alias!("U");
    add_alias!("V");
    add_alias!("W");
    add_alias!("X");
    add_alias!("Y");
    add_alias!("Z");
    add_alias!("Kc1" = "1");
    add_alias!("Kc2" = "2");
    add_alias!("Kc3" = "3");
    add_alias!("Kc4" = "4");
    add_alias!("Kc5" = "5");
    add_alias!("Kc6" = "6");
    add_alias!("Kc7" = "7");
    add_alias!("Kc8" = "8");
    add_alias!("Kc9" = "9");
    add_alias!("Kc0" = "0");
    add_alias!("Enter" = "ent");
    add_alias!("Escape" = "esc");
    add_alias!("Backspace" = "bspc");
    add_alias!("Tab");
    add_alias!("Space" = "spc");
    add_alias!("Minus" = "mins", "-");
    add_alias!("Equal" = "eql", "=");
    add_alias!("LeftBracket" = "left_bracket", "lbrc", "[");
    add_alias!("RightBracket" = "right_bracket", "rbrc", "]");
    add_alias!("Backslash" = "bsls", "\\");
    add_alias!("NonusHash" = "nonus_hash", "nuhs");
    add_alias!("Semicolon" = "scln", ";");
    add_alias!("Quote" = "quot", "'");
    add_alias!("Grave" = "grv", "`");
    add_alias!("Comma" = "comm", ",");
    add_alias!("Dot" = ".");
    add_alias!("Slash" = "slsh", "/");
    add_alias!("CapsLock" = "caps_lock", "caps");
    add_alias!("CapsWordToggle" = "caps_word", "cword");
    add_alias!("F1");
    add_alias!("F2");
    add_alias!("F3");
    add_alias!("F4");
    add_alias!("F5");
    add_alias!("F6");
    add_alias!("F7");
    add_alias!("F8");
    add_alias!("F9");
    add_alias!("F10");
    add_alias!("F11");
    add_alias!("F12");
    add_alias!("PrintScreen" = "print_screen", "pscr");
    add_alias!("ScrollLock" = "scroll_lock", "scrl", "brmd");
    add_alias!("Pause" = "paus", "brk", "brmu");
    add_alias!("Insert" = "ins");
    add_alias!("Home");
    add_alias!("PageUp" = "page_up", "pgup");
    add_alias!("Delete" = "del");
    add_alias!("End");
    add_alias!("PageDown" = "page_down", "pgdn");
    add_alias!("Right" = "rght");
    add_alias!("Left");
    add_alias!("Down");
    add_alias!("Up");
    add_alias!("NumLock" = "num_lock", "num");
    add_alias!("KpSlash" = "kp_slash", "psls");
    add_alias!("KpAsterisk" = "kp_asterisk", "past");
    add_alias!("KpMinus" = "kp_minus", "pmns");
    add_alias!("KpPlus" = "kp_plus", "ppls");
    add_alias!("KpEnter" = "kp_enter", "pent");
    add_alias!("Kp1" = "kp_1");
    add_alias!("Kp2" = "kp_2");
    add_alias!("Kp3" = "kp_3");
    add_alias!("Kp4" = "kp_4");
    add_alias!("Kp5" = "kp_5");
    add_alias!("Kp6" = "kp_6");
    add_alias!("Kp7" = "kp_7");
    add_alias!("Kp8" = "kp_8");
    add_alias!("Kp9" = "kp_9");
    add_alias!("Kp0" = "kp_0");
    add_alias!("KpDot" = "kp_dot", "pdot");
    add_alias!("NonusBackslash" = "nonus_backslash", "nubs");
    add_alias!("Application" = "app");
    add_alias!("KbPower" = "kb_power");
    add_alias!("KpEqual" = "kp_equal", "peql");
    add_alias!("F13");
    add_alias!("F14");
    add_alias!("F15");
    add_alias!("F16");
    add_alias!("F17");
    add_alias!("F18");
    add_alias!("F19");
    add_alias!("F20");
    add_alias!("F21");
    add_alias!("F22");
    add_alias!("F23");
    add_alias!("F24");
    add_alias!("Execute" = "exec");
    add_alias!("Help");
    add_alias!("Menu");
    add_alias!("Select" = "slct");
    add_alias!("Stop");
    add_alias!("Again" = "agin");
    add_alias!("Undo");
    add_alias!("Cut");
    add_alias!("Copy");
    add_alias!("Paste" = "pste");
    add_alias!("Find");
    add_alias!("KbMute" = "kb_mute");
    add_alias!("KbVolumeUp" = "kb_volume_up");
    add_alias!("KbVolumeDown" = "kb_volume_down");
    add_alias!("LockingCapsLock" = "locking_caps_lock", "lcap");
    add_alias!("LockingNumLock" = "locking_num_lock", "lnum");
    add_alias!("LockingScrollLock" = "locking_scroll_lock", "lscr");
    add_alias!("KpComma" = "kp_comma", "pcmm");
    add_alias!("KpEqualAs400" = "kp_equal_as400");
    add_alias!("International1" = "international_1", "int1");
    add_alias!("International2" = "international_2", "int2");
    add_alias!("International3" = "international_3", "int3");
    add_alias!("International4" = "international_4", "int4");
    add_alias!("International5" = "international_5", "int5");
    add_alias!("International6" = "international_6", "int6");
    add_alias!("International7" = "international_7", "int7");
    add_alias!("International8" = "international_8", "int8");
    add_alias!("International9" = "international_9", "int9");
    add_alias!("Language1" = "language_1", "lng1");
    add_alias!("Language2" = "language_2", "lng2");
    add_alias!("Language3" = "language_3", "lng3");
    add_alias!("Language4" = "language_4", "lng4");
    add_alias!("Language5" = "language_5", "lng5");
    add_alias!("Language6" = "language_6", "lng6");
    add_alias!("Language7" = "language_7", "lng7");
    add_alias!("Language8" = "language_8", "lng8");
    add_alias!("Language9" = "language_9", "lng9");
    add_alias!("AlternateErase" = "alternate_erase", "eras");
    add_alias!("SystemRequest" = "system_request", "syrq");
    add_alias!("Cancel" = "cncl");
    add_alias!("Clear" = "clr");
    add_alias!("Prior" = "prir");
    add_alias!("Return" = "retn");
    add_alias!("Separator" = "sepr");
    add_alias!("Out");
    add_alias!("Oper");
    add_alias!("ClearAgain" = "clear_again", "clag");
    add_alias!("Crsel" = "crsl");
    add_alias!("Exsel" = "exsl");
    add_alias!("SystemPower" = "system_power", "pwr");
    add_alias!("SystemSleep" = "system_sleep", "slep");
    add_alias!("SystemWake" = "system_wake", "wake");
    add_alias!("AudioMute" = "audio_mute", "mute");
    add_alias!("AudioVolUp" = "audio_vol_up", "volu");
    add_alias!("AudioVolDown" = "audio_vol_down", "vold");
    add_alias!("MediaNextTrack" = "media_next_track", "mnxt");
    add_alias!("MediaPrevTrack" = "media_prev_track", "mprv");
    add_alias!("MediaStop" = "media_stop", "mstp");
    add_alias!("MediaPlayPause" = "media_play_pause", "mply");
    add_alias!("MediaSelect" = "media_select", "msel");
    add_alias!("MediaEject" = "media_eject", "ejct");
    add_alias!("Mail");
    add_alias!("Calculator" = "calc");
    add_alias!("MyComputer" = "my_computer", "mycm");
    add_alias!("WwwSearch" = "www_search", "wsch");
    add_alias!("WwwHome" = "www_home", "whom");
    add_alias!("WwwBack" = "www_back", "wbak");
    add_alias!("WwwForward" = "www_forward", "wfwd");
    add_alias!("WwwStop" = "www_stop", "wstp");
    add_alias!("WwwRefresh" = "www_refresh", "wref");
    add_alias!("WwwFavorites" = "www_favorites", "wfav");
    add_alias!("MediaFastForward" = "media_fast_forward", "mffd");
    add_alias!("MediaRewind" = "media_rewind", "mrwd");
    add_alias!("BrightnessUp" = "brightness_up", "briu");
    add_alias!("BrightnessDown" = "brightness_down", "brid");
    add_alias!("ControlPanel" = "control_panel", "cpnl");
    add_alias!("Assistant" = "asst");
    add_alias!("MissionControl" = "mission_control", "mctl");
    add_alias!("Launchpad" = "lpad");
    add_alias!("AcRedo" = "ac_redo");
    add_alias!("SnapShot" = "snap_shot");
    add_alias!("MediaPlay" = "media_play");
    add_alias!("MediaPause" = "media_pause");
    add_alias!("MediaRandomPlay" = "media_random_play");
    add_alias!("MediaRepeat" = "media_repeat");
    add_alias!("MediaStopEject" = "media_stop_eject");
    add_alias!("ScreenLock" = "screen_lock");
    add_alias!("AcNew" = "ac_new");
    add_alias!("AcOpen" = "ac_open");
    add_alias!("AcClose" = "ac_close");
    add_alias!("AcExit" = "ac_exit");
    add_alias!("AcSave" = "ac_save");
    add_alias!("MouseUp" = "mousecursorup", "mouse_cursor_up", "ms_up");
    add_alias!("MouseDown" = "mousecursordown", "mouse_cursor_down", "ms_down");
    add_alias!("MouseLeft" = "mousecursorleft", "mouse_cursor_left", "ms_left");
    add_alias!("MouseRight" = "mousecursorright", "mouse_cursor_right", "ms_right");
    add_alias!("MouseBtn1" = "mouse_btn_1", "mousebutton1", "mouse_button_1", "ms_btn1");
    add_alias!("MouseBtn2" = "mouse_btn_2", "mousebutton2", "mouse_button_2", "ms_btn2");
    add_alias!("MouseBtn3" = "mouse_btn_3", "mousebutton3", "mouse_button_3", "ms_btn3");
    add_alias!("MouseBtn4" = "mouse_btn_4", "mousebutton4", "mouse_button_4", "ms_btn4");
    add_alias!("MouseBtn5" = "mouse_btn_5", "mousebutton5", "mouse_button_5", "ms_btn5");
    add_alias!("MouseBtn6" = "mouse_btn_6", "mousebutton6", "mouse_button_6", "ms_btn6");
    add_alias!("MouseBtn7" = "mouse_btn_7", "mousebutton7", "mouse_button_7", "ms_btn7");
    add_alias!("MouseBtn8" = "mouse_btn_8", "mousebutton8", "mouse_button_8", "ms_btn8");
    add_alias!("MouseWheelUp" = "mouse_wheel_up", "ms_whlu");
    add_alias!("MouseWheelDown" = "mouse_wheel_down", "ms_whld");
    add_alias!("MouseWheelLeft" = "mouse_wheel_left", "ms_whll");
    add_alias!("MouseWheelRight" = "mouse_wheel_right", "ms_whlr");
    add_alias!(
        "MouseAccel0" = "mouse_accel_0",
        "mouseacceleration0",
        "mouse_acceleration_0",
        "ms_acl0"
    );
    add_alias!(
        "MouseAccel1" = "mouse_accel_1",
        "mouseacceleration1",
        "mouse_acceleration_1",
        "ms_acl1"
    );
    add_alias!(
        "MouseAccel2" = "mouse_accel_2",
        "mouseacceleration2",
        "mouse_acceleration_2",
        "ms_acl2"
    );
    add_alias!("LCtrl" = "l_ctrl", "leftctrl", "left_ctrl", "lctl");
    add_alias!("LShift" = "l_shift", "leftshift", "left_shift", "lsft");
    add_alias!("LAlt" = "l_alt", "leftalt", "left_alt", "lopt");
    add_alias!("LGui" = "l_gui", "leftgui", "left_gui", "lcmd", "lwin");
    add_alias!("RCtrl" = "r_ctrl", "rightctrl", "right_ctrl", "rctl");
    add_alias!("RShift" = "r_shift", "rightshift", "right_shift", "rsft");
    add_alias!("RAlt" = "r_alt", "rightalt", "right_alt", "ropt", "algr");
    add_alias!("RGui" = "r_gui", "rightgui", "right_gui", "rcmd", "rwin");

    m
});
