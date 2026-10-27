//! Component values for each amplifier.
//!
//! Where a value comes from a factory schematic or a measured amp, it's
//! used as is. Where the original documentation isn't available (noted on
//! each amp), the circuit is the one that amp family used, with the values
//! typical of it.

use super::model::*;
use super::net::{self, Taper};
use super::output::{SpeakerSpec, TransformerSpec};
use super::tube::*;

fn ratio(z_spk: f64, raa: f64) -> f64 {
    (z_spk / raa).sqrt()
}

fn v_full(watts: f64, raa: f64) -> f64 {
    (2.0 * watts * raa).sqrt()
}

// ------------------------------------------------------------------ Vox --

const VOX_PRE: VoxPreSpec = VoxPreSpec {
    vb: 275.0,
    v1_ra: 220e3,
    // 1.5k / 25uF shared by both triodes of V1
    v1_rk: 3.0e3,
    v1_ck: 12.5e-6,
    tb_ra: 220e3,
    tb_rk: 1.5e3,
    tb_ck: 25e-6,
    cf_rk: 100e3,
    stack: net::VOX_TB,
    normal: false,
};

const BLUE: SpeakerSpec = SpeakerSpec { f_res: 75.0, z_peak: 5.0, q_res: 2.5, f_coil: 1200.0 };

/// Vox AC30/6 Top Boost, 1962-63: Brilliant channel through the Top Boost
/// circuit, four EL84s cathode-biased with no negative feedback, GZ34
/// rectifier, Cut control across the output grids.
pub fn ac30() -> AmpSpec {
    let raa = 4000.0;
    AmpSpec {
        pre: PreSpec::Vox(VOX_PRE),
        pi: PiSpec { valve: ECC83, vb: 300.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: Some((4.7e-9, 1e6)) },
        power: PowerSpec {
            valve: EL84,
            per_side: 2,
            b_plus: 355.0,
            screen_ratio: 0.92,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Cathode { rk: 50.0, ck: 100e-6 },
            r_supply: 170.0,
            c_supply: 32e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.8e3,
        },
        alt_rk: Some(82.0),
        nfb: 0.0,
        ot: TransformerSpec { f_low: 45.0, f_sat: 55.0, v_full: v_full(30.0, raa), f_high: 14000.0, q_high: 0.8, ratio: ratio(8.0, raa) },
        speaker: BLUE,
        current_drive: 0.85,
    }
}

/// Vox AC30/6, Normal channel: the same amp, played through the channel
/// with no Top Boost stage and no tone controls (Brian May's way, with a
/// treble booster in front).
pub fn ac30_normal() -> AmpSpec {
    AmpSpec { pre: PreSpec::Vox(VoxPreSpec { normal: true, ..VOX_PRE }), ..ac30() }
}

/// Vox AC50 Mk1, 1963-64 (typical values): Top Boost-style preamp, two
/// EL34s fixed bias, GZ34, light feedback.
pub fn ac50() -> AmpSpec {
    let raa = 3500.0;
    AmpSpec {
        pre: PreSpec::Vox(VoxPreSpec { vb: 300.0, ..VOX_PRE }),
        pi: PiSpec { valve: ECC83, vb: 330.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: None },
        power: PowerSpec {
            valve: EL34,
            per_side: 1,
            b_plus: 430.0,
            screen_ratio: 0.94,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 40.0 },
            r_supply: 150.0,
            c_supply: 32e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: 0.02,
        ot: TransformerSpec { f_low: 35.0, f_sat: 40.0, v_full: v_full(50.0, raa), f_high: 15000.0, q_high: 0.75, ratio: ratio(8.0, raa) },
        speaker: BLUE,
        current_drive: 0.85,
    }
}

/// Vox AC100, 1964-65 (typical values): four EL34s fixed bias, silicon
/// rectifier (stiff supply), light feedback.
pub fn ac100() -> AmpSpec {
    let raa = 1700.0;
    AmpSpec {
        pre: PreSpec::Vox(VoxPreSpec { vb: 300.0, ..VOX_PRE }),
        pi: PiSpec { valve: ECC83, vb: 330.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: None },
        power: PowerSpec {
            valve: EL34,
            per_side: 2,
            b_plus: 480.0,
            screen_ratio: 0.92,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 40.0 },
            r_supply: 50.0,
            c_supply: 64e-6,
            cc: 0.1e-6,
            rg: 100e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: 0.02,
        ot: TransformerSpec { f_low: 30.0, f_sat: 35.0, v_full: v_full(100.0, raa), f_high: 16000.0, q_high: 0.75, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 70.0, ..BLUE },
        current_drive: 0.85,
    }
}

/// Vox UL730, 1966 (typical values): transistor preamp with the Top Boost
/// tone network, four EL84s in ultra-linear.
pub fn ul730() -> AmpSpec {
    let raa = 4000.0;
    AmpSpec {
        pre: PreSpec::Hybrid(HybridPreSpec { gain1: 8.0, rails1: (4.5, 3.0), gain2: 20.0, rails2: (8.0, 6.0), stack: net::VOX_TB }),
        pi: PiSpec { valve: ECC83, vb: 300.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: None },
        power: PowerSpec {
            valve: EL84,
            per_side: 2,
            b_plus: 380.0,
            screen_ratio: 1.0,
            ultralinear: 0.43,
            raa,
            bias: PowerBias::Cathode { rk: 110.0, ck: 100e-6 },
            r_supply: 60.0,
            c_supply: 50e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: 0.04,
        ot: TransformerSpec { f_low: 40.0, f_sat: 50.0, v_full: v_full(30.0, raa), f_high: 14000.0, q_high: 0.75, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 80.0, ..BLUE },
        current_drive: 0.55,
    }
}

// --------------------------------------------------------------- Fender --

const AB763_PRE: FenderPreSpec = FenderPreSpec {
    v1: ECC83,
    vb: 270.0,
    stack: net::AB763_TWIN,
    fixed_mid: false,
    treble_taper: Taper::Audio,
    bass_taper: Taper::Audio,
    bright_cap: 120e-12,
    bright_switch: true,
    // 820R / 25uF shared by the two second-stage triodes
    v2_rk: 1.64e3,
    v2_ck: 12.5e-6,
    mix_cap: 0.047e-6,
};

/// Feedback divider 820R / 47R from the speaker output.
const AB763_NFB: f64 = 47.0 / (820.0 + 47.0);

fn ab763_pi(vb: f64) -> PiSpec {
    PiSpec { valve: ECC81, vb, ra1: 82e3, ra2: 100e3, rtail: 22e3, rbias: 470.0, cut: None }
}

/// Fender Deluxe Reverb, AB763 (blackface, as used in 1968): normal
/// channel, fixed 6.8k middle, 47pF bright cap, two 6V6GTs fixed bias,
/// GZ34, 820R/47R feedback.
pub fn deluxe_reverb() -> AmpSpec {
    let raa = 6600.0;
    AmpSpec {
        pre: PreSpec::Fender(FenderPreSpec { stack: net::AB763_DELUXE, fixed_mid: true, bright_cap: 47e-12, bright_switch: false, ..AB763_PRE }),
        pi: ab763_pi(420.0),
        power: PowerSpec {
            valve: V6V6,
            per_side: 1,
            b_plus: 420.0,
            screen_ratio: 0.98,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 22.0 },
            r_supply: 180.0,
            c_supply: 16e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB,
        ot: TransformerSpec { f_low: 40.0, f_sat: 50.0, v_full: v_full(22.0, raa), f_high: 15000.0, q_high: 0.75, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 90.0, z_peak: 5.0, q_res: 2.5, f_coil: 1100.0 },
        current_drive: 0.85,
    }
}

/// Fender Twin Reverb, 1968 silverface (AB763 circuit): normal channel with
/// bright switch, four 6L6s fixed bias, silicon rectifier.
pub fn twin_reverb() -> AmpSpec {
    let raa = 2000.0;
    AmpSpec {
        pre: PreSpec::Fender(AB763_PRE),
        pi: ab763_pi(450.0),
        power: PowerSpec {
            valve: L6L6,
            per_side: 2,
            b_plus: 470.0,
            screen_ratio: 0.97,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 40.0 },
            r_supply: 25.0,
            c_supply: 100e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB,
        ot: TransformerSpec { f_low: 25.0, f_sat: 30.0, v_full: v_full(85.0, raa), f_high: 20000.0, q_high: 0.7, ratio: ratio(4.0, raa) },
        speaker: SpeakerSpec { f_res: 60.0, z_peak: 7.0, q_res: 3.0, f_coil: 2500.0 },
        current_drive: 0.85,
    }
}

/// Fender Showman, AB763 blackface: like the Twin's circuit with two 6L6s.
pub fn showman() -> AmpSpec {
    let raa = 4000.0;
    AmpSpec {
        pre: PreSpec::Fender(AB763_PRE),
        pi: ab763_pi(450.0),
        power: PowerSpec {
            valve: L6L6,
            per_side: 1,
            b_plus: 465.0,
            screen_ratio: 0.97,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 38.0 },
            r_supply: 35.0,
            c_supply: 70e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB,
        ot: TransformerSpec { f_low: 25.0, f_sat: 30.0, v_full: v_full(85.0, raa), f_high: 20000.0, q_high: 0.7, ratio: ratio(4.0, raa) },
        speaker: SpeakerSpec { f_res: 40.0, z_peak: 8.0, q_res: 3.0, f_coil: 2500.0 },
        current_drive: 0.85,
    }
}

/// Fender Bassman 6G6-B blonde piggyback (typical values): 7025 input, deep
/// 1M bass pot, two 6L6s fixed bias, silicon rectifier.
pub fn bassman() -> AmpSpec {
    let raa = 4000.0;
    AmpSpec {
        pre: PreSpec::Fender(FenderPreSpec { v1: V7025, stack: net::BASSMAN_6G6, fixed_mid: true, ..AB763_PRE }),
        pi: PiSpec { valve: V7025, vb: 430.0, ra1: 82e3, ra2: 100e3, rtail: 10e3, rbias: 470.0, cut: None },
        power: PowerSpec {
            valve: L6L6,
            per_side: 1,
            b_plus: 440.0,
            screen_ratio: 0.97,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 35.0 },
            r_supply: 40.0,
            c_supply: 70e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB,
        ot: TransformerSpec { f_low: 30.0, f_sat: 35.0, v_full: v_full(40.0, raa), f_high: 18000.0, q_high: 0.7, ratio: ratio(4.0, raa) },
        speaker: SpeakerSpec { f_res: 85.0, z_peak: 6.0, q_res: 3.0, f_coil: 1000.0 },
        current_drive: 0.85,
    }
}

// ------------------------------------------------------------ additions --

/// Music Man 212 HD-130, late 70s (typical values): a solid-state preamp
/// with the Fender-style tone stack driving four 6CA7/EL34s at high voltage:
/// huge clean headroom, Mark Knopfler's amp from 1978 to the early 80s.
pub fn music_man_hd130() -> AmpSpec {
    let raa = 1900.0;
    AmpSpec {
        pre: PreSpec::Hybrid(HybridPreSpec { gain1: 6.0, rails1: (5.0, 4.0), gain2: 16.0, rails2: (9.0, 7.0), stack: net::AB763_TWIN }),
        pi: PiSpec { valve: ECC83, vb: 330.0, ra1: 82e3, ra2: 100e3, rtail: 22e3, rbias: 470.0, cut: None },
        power: PowerSpec {
            valve: EL34,
            per_side: 2,
            b_plus: 600.0,
            screen_ratio: 0.85,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 35.0 },
            r_supply: 30.0,
            c_supply: 100e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB,
        ot: TransformerSpec { f_low: 25.0, f_sat: 30.0, v_full: v_full(130.0, raa), f_high: 18000.0, q_high: 0.7, ratio: ratio(4.0, raa) },
        speaker: SpeakerSpec { f_res: 70.0, z_peak: 6.0, q_res: 3.0, f_coil: 2000.0 },
        current_drive: 0.85,
    }
}

/// Matchless DC30, 1990s (typical values): a Vox-family EL84 class A amp
/// with a Top Boost-like channel, cathode bias, no feedback.
pub fn matchless_dc30() -> AmpSpec {
    let raa = 4000.0;
    AmpSpec {
        pre: PreSpec::Vox(VoxPreSpec { vb: 290.0, tb_rk: 1.2e3, ..VOX_PRE }),
        pi: PiSpec { valve: ECC83, vb: 310.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: Some((4.7e-9, 1e6)) },
        power: PowerSpec {
            valve: EL84,
            per_side: 2,
            b_plus: 345.0,
            screen_ratio: 0.95,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Cathode { rk: 55.0, ck: 250e-6 },
            r_supply: 200.0,
            c_supply: 40e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: 0.0,
        ot: TransformerSpec { f_low: 40.0, f_sat: 50.0, v_full: v_full(30.0, raa), f_high: 15000.0, q_high: 0.8, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 80.0, z_peak: 5.0, q_res: 2.5, f_coil: 1300.0 },
        current_drive: 0.85,
    }
}

/// Magnatone as modified for Duane Eddy (estimated): a Fender-style preamp
/// into a 100-watt 6L6 power pack, stiff supply, extra feedback: loud and
/// clean however hard it's played.
pub fn magnatone_custom() -> AmpSpec {
    let raa = 2000.0;
    AmpSpec {
        pre: PreSpec::Fender(FenderPreSpec { bright_switch: false, bright_cap: 47e-12, ..AB763_PRE }),
        pi: ab763_pi(460.0),
        power: PowerSpec {
            valve: L6L6,
            per_side: 2,
            b_plus: 480.0,
            screen_ratio: 0.95,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Fixed { idle_ma: 38.0 },
            r_supply: 20.0,
            c_supply: 120e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: AB763_NFB * 1.6,
        ot: TransformerSpec { f_low: 25.0, f_sat: 30.0, v_full: v_full(100.0, raa), f_high: 18000.0, q_high: 0.7, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 45.0, z_peak: 8.0, q_res: 3.0, f_coil: 2500.0 },
        current_drive: 0.85,
    }
}

/// Gibson EH-150, 1937-42 (typical values): a small two-stage preamp and
/// fixed tone, a pair of 6L6s cathode-biased at low voltage, a tube
/// rectifier and a 12-inch field-coil speaker: the jazz and western swing
/// amp of the Charlie Christian era.
pub fn eh150() -> AmpSpec {
    let raa = 5000.0;
    AmpSpec {
        pre: PreSpec::Fender(FenderPreSpec { v1: ECC83, stack: net::BASSMAN_6G6, fixed_mid: true, bright_cap: 0.0, bright_switch: false, ..AB763_PRE }),
        pi: PiSpec { valve: ECC83, vb: 300.0, ra1: 100e3, ra2: 100e3, rtail: 47e3, rbias: 1e3, cut: None },
        power: PowerSpec {
            valve: L6L6,
            per_side: 1,
            b_plus: 330.0,
            screen_ratio: 0.95,
            ultralinear: 0.0,
            raa,
            bias: PowerBias::Cathode { rk: 250.0, ck: 25e-6 },
            r_supply: 250.0,
            c_supply: 16e-6,
            cc: 0.05e-6,
            rg: 470e3,
            r_stop: 1.5e3,
        },
        alt_rk: None,
        nfb: 0.0,
        ot: TransformerSpec { f_low: 70.0, f_sat: 80.0, v_full: v_full(15.0, raa), f_high: 8000.0, q_high: 0.8, ratio: ratio(8.0, raa) },
        speaker: SpeakerSpec { f_res: 100.0, z_peak: 5.0, q_res: 2.5, f_coil: 900.0 },
        current_drive: 0.85,
    }
}

pub fn all() -> Vec<(&'static str, AmpSpec)> {
    vec![
        ("AC30 Top Boost", ac30()),
        ("AC50", ac50()),
        ("AC100", ac100()),
        ("UL730", ul730()),
        ("Bassman", bassman()),
        ("Showman", showman()),
        ("Deluxe Reverb", deluxe_reverb()),
        ("Twin Reverb", twin_reverb()),
        ("HD-130", music_man_hd130()),
        ("DC30", matchless_dc30()),
        ("Magnatone", magnatone_custom()),
        ("EH-150", eh150()),
    ]
}
