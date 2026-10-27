//! Built-in loudspeaker cabinets: a close microphone on each period cab,
//! modelled as its measured-style frequency response (box resonance,
//! cone break-up peaks, top-end roll-off) rather than a sampled IR.

use crate::dsp::Biquad;

#[derive(Clone, Copy, Debug)]
pub struct CabSpec {
    pub name: &'static str,
    /// Low cut from the box/speaker resonance.
    pub f_lo: f64,
    pub q_lo: f64,
    /// Open-back cabinets lose more deep bass (front/back cancellation).
    pub open_back: bool,
    /// (frequency, Q, dB) peaks and dips.
    pub peaks: [(f64, f64, f64); 4],
    /// High-frequency roll-off (4th order).
    pub f_hi: f64,
    /// Horn tweeters: (shelf frequency, dB) added before the roll-off.
    pub horn: Option<(f64, f64)>,
}

pub const VOX_2X12_BLUE: CabSpec = CabSpec {
    name: "Vox 2x12 open back, Celestion Alnico Blue",
    f_lo: 82.0,
    q_lo: 0.8,
    open_back: true,
    peaks: [(125.0, 1.2, 2.5), (420.0, 1.0, -2.5), (2300.0, 1.3, 5.0), (4100.0, 2.2, 3.0)],
    f_hi: 5600.0,
    horn: None,
};

pub const VOX_2X12_HORN: CabSpec = CabSpec {
    name: "Vox 2x12 + Midax horn, closed back",
    f_lo: 72.0,
    q_lo: 1.0,
    open_back: false,
    peaks: [(140.0, 1.0, 2.0), (500.0, 1.0, -2.0), (2500.0, 1.2, 4.0), (3600.0, 2.0, 1.5)],
    f_hi: 9000.0,
    horn: Some((4200.0, 6.0)),
};

pub const VOX_4X12_HORNS: CabSpec = CabSpec {
    name: "Vox AC100 4x12 + 2 horns, closed back",
    f_lo: 68.0,
    q_lo: 1.1,
    open_back: false,
    peaks: [(110.0, 1.0, 3.0), (480.0, 1.0, -2.0), (2500.0, 1.2, 4.0), (3600.0, 2.0, 1.5)],
    f_hi: 9500.0,
    horn: Some((4500.0, 5.0)),
};

pub const FENDER_2X12_JENSEN: CabSpec = CabSpec {
    name: "Fender Bassman 2x12 closed back, Jensen C12N",
    f_lo: 75.0,
    q_lo: 1.0,
    open_back: false,
    peaks: [(120.0, 1.1, 3.0), (520.0, 1.0, -2.0), (2000.0, 1.2, 4.0), (3500.0, 2.0, 2.0)],
    f_hi: 5000.0,
    horn: None,
};

pub const FENDER_2X12_OXFORD: CabSpec = CabSpec {
    name: "Fender Bassman 2x12 closed back, Oxford 12K5 (estimated)",
    f_lo: 78.0,
    q_lo: 1.0,
    open_back: false,
    peaks: [(125.0, 1.1, 2.5), (550.0, 1.0, -1.5), (1900.0, 1.2, 4.5), (3300.0, 2.0, 3.0)],
    f_hi: 5200.0,
    horn: None,
};

pub const FENDER_2X15_JBL: CabSpec = CabSpec {
    name: "Fender Showman 2x15 closed back, JBL D130F",
    f_lo: 58.0,
    q_lo: 0.9,
    open_back: false,
    peaks: [(90.0, 1.0, 2.0), (600.0, 1.0, -1.5), (3000.0, 1.0, 5.0), (5000.0, 2.0, 3.0)],
    f_hi: 7500.0,
    horn: None,
};

pub const FENDER_1X12_OXFORD: CabSpec = CabSpec {
    name: "Fender Deluxe Reverb 1x12 open back, Oxford 12K5",
    f_lo: 95.0,
    q_lo: 0.8,
    open_back: true,
    peaks: [(200.0, 1.0, 2.0), (650.0, 1.0, -1.5), (1800.0, 1.2, 4.0), (3200.0, 2.0, 3.0)],
    f_hi: 5000.0,
    horn: None,
};

pub const FENDER_2X12_JBL: CabSpec = CabSpec {
    name: "Fender Twin Reverb 2x12 open back, JBL D120F",
    f_lo: 85.0,
    q_lo: 0.8,
    open_back: true,
    peaks: [(150.0, 1.0, 2.0), (550.0, 1.0, -1.5), (2800.0, 1.1, 4.0), (5000.0, 2.0, 3.0)],
    f_hi: 8000.0,
    horn: None,
};

pub const VOX_UL_2X12: CabSpec = CabSpec {
    name: "Vox UL 2x12, Celestion",
    f_lo: 80.0,
    q_lo: 0.9,
    open_back: true,
    peaks: [(130.0, 1.0, 2.0), (450.0, 1.0, -2.0), (1800.0, 1.2, 3.0), (2800.0, 1.5, 4.0)],
    f_hi: 5500.0,
    horn: None,
};

// ---- more cabinets (estimated responses from published curves and typical close-mic shots)

macro_rules! cab {
    ($id:ident, $name:expr, $flo:expr, $qlo:expr, $open:expr, $peaks:expr, $fhi:expr, $horn:expr) => {
        pub const $id: CabSpec = CabSpec { name: $name, f_lo: $flo, q_lo: $qlo, open_back: $open, peaks: $peaks, f_hi: $fhi, horn: $horn };
    };
}

cab!(FENDER_4X10_P10R, "Fender Bassman 4x10 open back, Jensen P10R", 70.0, 0.9, true, [(110.0, 1.0, 2.0), (500.0, 1.0, -1.0), (2200.0, 1.2, 4.0), (4000.0, 2.0, 2.5)], 6500.0, None);
cab!(MARSHALL_4X12_GREENBACK, "Marshall 1960 4x12 closed back, Celestion G12M Greenback", 75.0, 1.1, false, [(110.0, 1.0, 3.5), (400.0, 1.0, -2.0), (1800.0, 1.2, 3.0), (2800.0, 2.0, 4.0)], 5200.0, None);
cab!(MARSHALL_4X12_G12T75, "Marshall 1960A 4x12 closed back, Celestion G12T-75", 72.0, 1.0, false, [(105.0, 1.0, 3.0), (450.0, 1.0, -3.0), (2600.0, 1.3, 5.0), (4400.0, 2.0, 3.0)], 5800.0, None);
cab!(HIWATT_4X12_FANE, "Hiwatt SE4123 4x12 closed back, Fane", 70.0, 1.0, false, [(100.0, 1.0, 3.0), (500.0, 1.0, -1.0), (2000.0, 1.2, 2.5), (3800.0, 2.0, 2.0)], 7000.0, None);
cab!(MESA_4X12_V30, "Mesa/Boogie 4x12 closed back, Celestion Vintage 30", 72.0, 1.1, false, [(115.0, 1.0, 3.0), (400.0, 1.0, -2.0), (2600.0, 1.6, 6.0), (4600.0, 2.5, 4.0)], 5600.0, None);
cab!(MESA_1X12_EV, "Mesa/Boogie Mark 1x12 open back, Electro-Voice EVM12L", 85.0, 0.9, true, [(160.0, 1.0, 2.0), (600.0, 1.0, -1.0), (2200.0, 1.2, 3.0), (4500.0, 2.0, 2.0)], 7500.0, None);
cab!(MATCHLESS_2X12, "Matchless 2x12 open back, Celestion G12H30 + Alnico Blue", 82.0, 0.8, true, [(130.0, 1.2, 2.5), (450.0, 1.0, -2.0), (2300.0, 1.3, 4.5), (4000.0, 2.2, 3.0)], 6000.0, None);
cab!(MUSICMAN_2X12, "Music Man 212 open back, 12-inch", 85.0, 0.8, true, [(150.0, 1.0, 2.0), (550.0, 1.0, -1.5), (2600.0, 1.1, 4.0), (5000.0, 2.0, 3.0)], 8000.0, None);
cab!(MAGNATONE_1X15_HORN, "Modified Magnatone 1x15 JBL + tweeter", 60.0, 0.9, false, [(90.0, 1.0, 2.0), (600.0, 1.0, -1.0), (2800.0, 1.0, 4.0), (5000.0, 2.0, 2.0)], 7500.0, Some((5000.0, 6.0)));
cab!(EH150_1X12, "Gibson EH-150 1x12 open back, field-coil Jensen", 100.0, 0.8, true, [(220.0, 1.0, 2.5), (700.0, 1.0, -1.0), (1600.0, 1.2, 3.0), (2800.0, 2.0, 2.0)], 4200.0, None);
cab!(AMPEG_8X10, "Ampeg SVT 8x10 closed back", 50.0, 1.1, false, [(80.0, 1.0, 3.0), (400.0, 1.0, -2.0), (1500.0, 1.2, 3.0), (3000.0, 2.0, -2.0)], 4500.0, None);
cab!(AMPEG_B15_1X15, "Ampeg B-15 Portaflex 1x15", 45.0, 1.2, false, [(70.0, 1.1, 4.0), (350.0, 1.0, -2.0), (1200.0, 1.2, 2.0), (2500.0, 2.0, -3.0)], 3500.0, None);
cab!(WIZARD_2X12_V30, "Wizard 2x12 closed back, Celestion Vintage 30", 80.0, 1.0, false, [(125.0, 1.1, 2.5), (450.0, 1.0, -2.0), (2500.0, 1.5, 5.5), (4500.0, 2.5, 3.5)], 5800.0, None);

/// A close-miked cabinet. `mic` moves the microphone from the cone centre
/// (0, bright and direct) out toward the edge (1, darker and rounder).
#[derive(Clone)]
pub struct Cabinet {
    fs: f64,
    spec: CabSpec,
    lo: Biquad,
    open: Biquad,
    peaks: [Biquad; 4],
    /// The same cabinet heard without a close mic (see `process_speaker`).
    far_peaks: [Biquad; 4],
    far_tilt: Biquad,
    horn: Biquad,
    lp: [Biquad; 2],
    mic_shelf: Biquad,
    mic_dip: Biquad,
    mic: f64,
}

impl Cabinet {
    pub fn new(fs: f64, spec: CabSpec) -> Self {
        let mut c = Self {
            fs,
            spec,
            lo: Biquad::identity(),
            open: Biquad::identity(),
            peaks: [Biquad::identity(), Biquad::identity(), Biquad::identity(), Biquad::identity()],
            far_peaks: [Biquad::identity(), Biquad::identity(), Biquad::identity(), Biquad::identity()],
            far_tilt: Biquad::identity(),
            horn: Biquad::identity(),
            lp: [Biquad::identity(), Biquad::identity()],
            mic_shelf: Biquad::identity(),
            mic_dip: Biquad::identity(),
            mic: -1.0,
        };
        c.lo.set_highpass(fs, spec.f_lo, spec.q_lo);
        if spec.open_back {
            c.open.set_low_shelf(fs, spec.f_lo * 1.6, 0.7, -4.0);
        }
        for (b, &(f, q, g)) in c.peaks.iter_mut().zip(&spec.peaks) {
            b.set_peak(fs, f, q, g);
        }
        // Away from the cone the narrow on-axis presence peaks average out
        // (a room mic hears the cabinet's power response), and the highs
        // beam: keep the box and cone character, soften the close-mic
        // presence, tilt the very top down a little.
        for (b, &(f, q, g)) in c.far_peaks.iter_mut().zip(&spec.peaks) {
            let g = if f > 1500.0 { g * 0.55 } else { g };
            b.set_peak(fs, f, q * 0.8, g);
        }
        c.far_tilt.set_high_shelf(fs, 3500.0, 0.7, -2.0);
        if let Some((f, g)) = spec.horn {
            c.horn.set_high_shelf(fs, f, 0.8, g);
        }
        // Butterworth 4th order: Q 0.541 and 1.307
        c.lp[0].set_lowpass(fs, spec.f_hi, 0.541);
        c.lp[1].set_lowpass(fs, spec.f_hi, 1.307);
        c.set_mic(0.3);
        c
    }

    pub fn spec(&self) -> &CabSpec {
        &self.spec
    }

    pub fn set_mic(&mut self, mic: f64) {
        let mic = mic.clamp(0.0, 1.0);
        if (mic - self.mic).abs() < 1e-4 {
            return;
        }
        self.mic = mic;
        self.mic_shelf.set_high_shelf(self.fs, 2400.0, 0.7, -9.0 * mic);
        self.mic_dip.set_peak(self.fs, 4200.0, 1.5, -3.0 * mic + 1.5 * (1.0 - mic));
    }

    pub fn reset(&mut self) {
        self.lo.reset();
        self.open.reset();
        self.peaks.iter_mut().for_each(|b| b.reset());
        self.far_peaks.iter_mut().for_each(|b| b.reset());
        self.far_tilt.reset();
        self.horn.reset();
        self.lp.iter_mut().for_each(|b| b.reset());
        self.mic_shelf.reset();
        self.mic_dip.reset();
    }

    /// The cabinet with no close microphone: what the speaker puts into the
    /// room, for miking it up with Studio Spaces or the Mic plugin.
    #[inline]
    pub fn process_speaker(&mut self, x: f64) -> f64 {
        let mut y = self.lo.process(x);
        if self.spec.open_back {
            y = self.open.process(y);
        }
        for b in &mut self.far_peaks {
            y = b.process(y);
        }
        if self.spec.horn.is_some() {
            y = self.horn.process(y);
        }
        y = self.far_tilt.process(y);
        let y = self.lp[0].process(y);
        self.lp[1].process(y)
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let mut y = self.lo.process(x);
        if self.spec.open_back {
            y = self.open.process(y);
        }
        for b in &mut self.peaks {
            y = b.process(y);
        }
        if self.spec.horn.is_some() {
            y = self.horn.process(y);
        }
        y = self.mic_shelf.process(y);
        y = self.mic_dip.process(y);
        let y = self.lp[0].process(y);
        self.lp[1].process(y)
    }
}
