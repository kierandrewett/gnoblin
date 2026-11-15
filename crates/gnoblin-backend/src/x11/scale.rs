//! The scale X11 clients are told, and what XSETTINGS says about it.
//!
//! An output composed at `mode * s` with the user's scale `u` is shown to Wayland clients at `u * s`, so a client
//! draws `u * s` pixels for every logical one. X11 has no scale protocol: Xwayland's own `wl_surface`s are the only
//! place a compositor can say "this client's pixels are `1 / k` of a logical pixel", and XSETTINGS is the only place it
//! can ask a toolkit to draw `k` times as many of them. [`client_scale`] decides `k`; [`with_scale`] puts the toolkit
//! half into the settings the desktop publishes, in the convention `gnome-settings-daemon` has always used: `Xft/DPI`
//! is the DPI times the window scale, `Gdk/UnscaledDPI` is the DPI before it, and `Gdk/WindowScalingFactor` is the
//! window scale.
//!
//! # Failure modes first
//!
//! 1. **An output nothing is told about makes the scale one.** No outputs, no scale to follow: the answer is the fixed
//!    scale or one, so a session that has not seen an output yet starts Xwayland exactly as before:
//!    `no_output_is_no_scale`.
//! 2. **A fixed scale is lost under supersampling.** The user's `scaling-factor` stays what decides how big X11 windows
//!    are; the factor multiplies it so the same window has the same logical size and `s` times the pixels:
//!    `a_fixed_scale_is_multiplied_by_the_largest_factor`.
//! 3. **The scale of one output is applied to a window on another.** Xwayland has one scale per client, so the answer
//!    is the largest of the outputs', which is the one no output shows blurred: `the_largest_output_decides`.
//! 4. **A toolkit is told a scale it cannot draw.** GTK 3 takes an integer window scale, so
//!    `Gdk/WindowScalingFactor` is the nearest whole number (never below one) while the DPI keeps the exact fraction:
//!    `the_window_scale_is_a_whole_number_and_the_dpi_keeps_the_fraction`.
//! 5. **No scale changes nothing.** At a scale of one the settings are the ones the desktop published, untouched, so a
//!    session without supersampling or scaling publishes what it always did: `scale_one_leaves_the_settings_alone`.

use gnoblin_types::Scale;

use super::Setting;

/// The XSETTINGS name of the DPI toolkits read, in thousandths-and-twenty-fourths of a dot per inch.
pub const DPI: &str = "Xft/DPI";

/// The XSETTINGS name of the DPI before the window scale is applied to it.
pub const UNSCALED_DPI: &str = "Gdk/UnscaledDPI";

/// The XSETTINGS name of the integer window scale GTK reads.
pub const WINDOW_SCALING_FACTOR: &str = "Gdk/WindowScalingFactor";

/// 96 dots per inch in XSETTINGS units: what a toolkit assumes when the desktop publishes none.
const DEFAULT_DPI: i32 = 96 * 1024;

/// What one output contributes to the scale X11 clients are told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutputScale {
    /// The user's scale of the output, before supersampling.
    pub scale: Scale,
    /// The supersample factor of the output; one is none.
    pub supersample: Scale,
}

/// The scale Xwayland's client is given, for the outputs there are.
///
/// `fixed` is the `xwayland.scaling-factor` the user set, or [`None`] for automatic. Automatic follows the outputs: the
/// largest `scale * supersample` among them. A fixed scale is the user's own choice of how big X11 windows are, so it
/// is kept, and only multiplied by the largest supersample factor so that a window keeps its logical size while its
/// pixels become `s` times as many.
#[must_use]
pub fn client_scale(fixed: Option<Scale>, outputs: &[OutputScale]) -> Scale {
    match fixed {
        Some(fixed) => {
            let factor = outputs.iter().map(|output| output.supersample).max();
            factor.map_or(fixed, |factor| fixed.product(factor))
        }
        None => outputs
            .iter()
            .map(|output| output.scale.product(output.supersample))
            .max()
            .unwrap_or(Scale::IDENTITY),
    }
}

/// The integer window scale a toolkit is told for a client scale: the nearest whole number, never below one.
#[must_use]
pub fn window_scaling_factor(client: Scale) -> i32 {
    let (numerator, denominator) = (u64::from(client.numerator()), u64::from(client.denominator()));
    let rounded = (2 * numerator + denominator) / (2 * denominator);
    i32::try_from(rounded).unwrap_or(i32::MAX).max(1)
}

/// `settings` as an X11 client at `client` should see them.
///
/// A scale of one returns the list unchanged. Otherwise `Xft/DPI` becomes the desktop's DPI times the scale (96 when
/// the desktop published none), `Gdk/UnscaledDPI` keeps the DPI as it was, and `Gdk/WindowScalingFactor` is added. The
/// list stays in name order and a name appears once, as the seam requires.
#[must_use]
pub fn with_scale(settings: Vec<(String, Setting)>, client: Scale) -> Vec<(String, Setting)> {
    if client.is_identity() {
        return settings;
    }
    let mut settings: Vec<(String, Setting)> = settings
        .into_iter()
        .filter(|(name, _)| name != UNSCALED_DPI && name != WINDOW_SCALING_FACTOR)
        .collect();
    let unscaled = settings
        .iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (DPI, Setting::Integer(dpi)) => Some(*dpi),
            _ => None,
        })
        .unwrap_or(DEFAULT_DPI);
    let (numerator, denominator) = (i64::from(client.numerator()), i64::from(client.denominator()));
    let scaled = i32::try_from((i64::from(unscaled) * numerator + denominator / 2) / denominator).unwrap_or(i32::MAX);
    settings.retain(|(name, _)| name != DPI);
    settings.push((DPI.to_owned(), Setting::Integer(scaled)));
    settings.push((UNSCALED_DPI.to_owned(), Setting::Integer(unscaled)));
    settings.push((
        WINDOW_SCALING_FACTOR.to_owned(),
        Setting::Integer(window_scaling_factor(client)),
    ));
    settings.sort_by(|left, right| left.0.cmp(&right.0));
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn output(scale: u32, supersample: u32) -> OutputScale {
        OutputScale {
            scale: Scale::integral(scale),
            supersample: Scale::integral(supersample),
        }
    }

    #[test]
    fn no_output_is_no_scale() {
        assert_eq!(client_scale(None, &[]), Scale::IDENTITY);
        assert_eq!(client_scale(Some(Scale::new(3, 2)), &[]), Scale::new(3, 2));
    }

    #[test]
    fn automatic_follows_the_outputs_scale_times_their_factor() {
        assert_eq!(client_scale(None, &[output(1, 1)]), Scale::IDENTITY);
        assert_eq!(client_scale(None, &[output(1, 2)]), Scale::integral(2));
        assert_eq!(client_scale(None, &[output(2, 2)]), Scale::integral(4));
        let fractional = OutputScale {
            scale: Scale::new(3, 2),
            supersample: Scale::integral(2),
        };
        assert_eq!(client_scale(None, &[fractional]), Scale::integral(3));
    }

    #[test]
    fn the_largest_output_decides() {
        assert_eq!(
            client_scale(None, &[output(1, 1), output(1, 4), output(2, 1)]),
            Scale::integral(4)
        );
    }

    #[test]
    fn a_fixed_scale_is_multiplied_by_the_largest_factor() {
        let fixed = Some(Scale::new(3, 2));
        assert_eq!(
            client_scale(fixed, &[output(1, 1)]),
            Scale::new(3, 2),
            "no factor, no change"
        );
        assert_eq!(
            client_scale(fixed, &[output(2, 1)]),
            Scale::new(3, 2),
            "a fixed scale ignores the output's own"
        );
        assert_eq!(client_scale(fixed, &[output(1, 2), output(1, 4)]), Scale::integral(6));
    }

    #[test]
    fn the_window_scale_is_a_whole_number_and_the_dpi_keeps_the_fraction() {
        assert_eq!(window_scaling_factor(Scale::IDENTITY), 1);
        assert_eq!(window_scaling_factor(Scale::new(3, 2)), 2);
        assert_eq!(window_scaling_factor(Scale::new(5, 4)), 1);
        assert_eq!(window_scaling_factor(Scale::integral(4)), 4);
        let published = with_scale(vec![(DPI.to_owned(), Setting::Integer(96 * 1024))], Scale::new(3, 2));
        assert!(
            published.contains(&(DPI.to_owned(), Setting::Integer(144 * 1024))),
            "{published:?}"
        );
        assert!(published.contains(&(UNSCALED_DPI.to_owned(), Setting::Integer(96 * 1024))));
        assert!(published.contains(&(WINDOW_SCALING_FACTOR.to_owned(), Setting::Integer(2))));
    }

    #[test]
    fn scale_one_leaves_the_settings_alone() {
        let settings = vec![("Gtk/ThemeName".to_owned(), Setting::Text("Adwaita".to_owned()))];
        assert_eq!(with_scale(settings.clone(), Scale::IDENTITY), settings);
        assert!(
            with_scale(Vec::new(), Scale::IDENTITY).is_empty(),
            "and no settings stay none"
        );
    }

    #[test]
    fn a_scale_without_a_published_dpi_assumes_96() {
        let published = with_scale(Vec::new(), Scale::integral(2));
        assert_eq!(
            published,
            [
                (UNSCALED_DPI.to_owned(), Setting::Integer(96 * 1024)),
                (WINDOW_SCALING_FACTOR.to_owned(), Setting::Integer(2)),
                (DPI.to_owned(), Setting::Integer(192 * 1024)),
            ],
            "the three names, in name order"
        );
    }

    #[test]
    fn the_list_stays_in_name_order_and_keeps_the_desktops_other_settings() {
        let settings = vec![
            (DPI.to_owned(), Setting::Integer(110 * 1024)),
            ("Gtk/FontName".to_owned(), Setting::Text("Cantarell 11".to_owned())),
            (WINDOW_SCALING_FACTOR.to_owned(), Setting::Integer(1)),
        ];
        let published = with_scale(settings, Scale::integral(2));
        let names: Vec<&str> = published.iter().map(|(name, _)| name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "name order");
        sorted.dedup();
        assert_eq!(
            names.len(),
            sorted.len(),
            "a name once, so the desktop's own window scale does not duplicate"
        );
        assert!(published.contains(&("Gtk/FontName".to_owned(), Setting::Text("Cantarell 11".to_owned()))));
        assert!(published.contains(&(DPI.to_owned(), Setting::Integer(220 * 1024))));
        assert!(published.contains(&(UNSCALED_DPI.to_owned(), Setting::Integer(110 * 1024))));
        assert!(published.contains(&(WINDOW_SCALING_FACTOR.to_owned(), Setting::Integer(2))));
    }
}
