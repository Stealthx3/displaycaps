// SPDX-License-Identifier: LGPL-2.1-or-later
//! Compositor layer tests over hand-written facts (the lab fixtures cover the real desktops).

use super::facts::*;
use super::*;
use displaycaps_model::{Selection, SensitiveValue};

fn ep(id: &str) -> Endpoint {
    let p = Provenance {
        layer: LayerKind::HostHardware,
        source: Source::derived("test"),
        read_at_ms: 0,
    };
    Endpoint {
        id: EndpointId(id.into()),
        class: EndpointClass::External,
        connector: Value::measured(ConnectorType::Hdmi, p.clone()),
        status: Value::measured(ConnectionStatus::Connected, p),
        name: None,
        correlation: vec![],
        sink: None,
        layers: vec![],
        effective: vec![],
        paths: vec![],
    }
}

fn edid(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../tests/fixtures/edid-corpus/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn with_sink(mut e: Endpoint, raw: &[u8]) -> Endpoint {
    e.sink = crate::sink::sink_record(
        raw,
        Source::derived("test"),
        0,
        &Selection::All,
        &Default::default(),
    );
    e
}

fn conf<T>(v: &Value<T>) -> Option<Confidence> {
    match v {
        Value::Known { confidence, .. } => Some(*confidence),
        Value::Unknown { .. } => None,
    }
}

fn comp(e: &Endpoint) -> &LayerCapabilities {
    e.layers
        .iter()
        .find(|l| l.layer == LayerKind::Compositor)
        .expect("compositor layer")
}

fn detail(e: &Endpoint) -> &CompositorOutput {
    match &comp(e).detail {
        Some(LayerDetail::Compositor(c)) => c,
        other => panic!("no compositor detail: {other:?}"),
    }
}

fn globals(names: &[&str]) -> Vec<Global> {
    names
        .iter()
        .map(|n| Global {
            interface: n.to_string(),
            version: 1,
        })
        .collect()
}

fn pq_image() -> ImageInfo {
    ImageInfo {
        primaries_named: Some("bt2020".into()),
        tf_named: Some("st2084_pq".into()),
        luminances: Some([50, 10_000, 203]),
        target_primaries: Some([
            680_000, 320_000, 265_000, 690_000, 150_000, 60_000, 312_700, 329_000,
        ]),
        target_luminance: Some([50, 1000]),
        target_max_fall: Some(400),
        ..ImageInfo::default()
    }
}

fn mode(w: i32, h: i32, mhz: i32) -> WlMode {
    WlMode {
        width: w,
        height: h,
        refresh_mhz: mhz,
        preferred: false,
    }
}

fn kwin_hdr() -> CompositorFacts {
    CompositorFacts {
        wayland: Some(Read::Ok(WaylandFacts {
            globals: globals(&["wl_output", "wp_color_manager_v1", "kde_output_device_v2"]),
            outputs: vec![WlOutput {
                version: 4,
                name: Some("HDMI-A-1".into()),
                mode: Some(mode(3840, 2160, 60_000)),
                scale: Some(2),
                image_description: Some(Read::Ok(pq_image())),
                ..WlOutput::default()
            }],
            kde_devices: Some(vec![KdeDevice {
                version: 11,
                name: Some("HDMI-A-1".into()),
                enabled: Some(true),
                modes: vec![mode(3840, 2160, 60_000), mode(3840, 2160, 23_976)],
                current_mode: Some(0),
                scale: Some(1.5),
                capabilities: Some(kde_cap::VRR | kde_cap::RGB_RANGE | kde_cap::HDR | 0x10),
                vrr_policy: Some(2),
                rgb_range: Some(1),
                hdr: Some(true),
                wide_color: Some(true),
                sdr_brightness: Some(250),
                brightness_metadata: Some([1000, 400, 50]),
                brightness_overrides: Some([800, -1, -1]),
                ..KdeDevice::default()
            }]),
            ..WaylandFacts::default()
        })),
        ..CompositorFacts::default()
    }
}

#[test]
fn kwin_hdr_output_prefers_kwins_own_state() {
    let mut eps = vec![ep("card1-HDMI-A-1")];
    let scope = ProbeScope::everything();
    let a = attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(kwin_hdr()),
        &scope,
        7,
    );
    assert_eq!(a, Availability::Read);
    let d = detail(&eps[0]);
    assert_eq!(d.compositor, CompositorKind::Kwin);
    assert_eq!(d.hdr_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(
        conf(d.hdr_enabled.as_ref().unwrap()),
        Some(Confidence::Measured),
        "KWin states HDR itself; the image description only infers it"
    );
    assert_eq!(d.wide_color_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(
        d.sdr_brightness_nits.as_ref().unwrap().known(),
        Some(&250.0)
    );
    // KWin's fractional scale, not wl_output's integer buffer scale.
    assert_eq!(d.scale.as_ref().unwrap().known(), Some(&1.5));
    assert_eq!(
        d.vrr_policy.as_ref().unwrap().known(),
        Some(&VrrPolicy::Automatic)
    );
    assert_eq!(
        d.rgb_range.as_ref().unwrap().known().map(String::as_str),
        Some("full")
    );
    let hdr = d.hdr.as_ref().unwrap().known().unwrap();
    assert_eq!(hdr.eotfs, [Eotf::TraditionalSdr, Eotf::Pq]);
    assert_eq!(
        hdr.max_luminance_nits,
        Some(800.0),
        "the user's override wins"
    );
    assert_eq!(hdr.max_frame_average_nits, Some(400.0));
    assert_eq!(hdr.min_luminance_nits, Some(0.005));
    let img = d.image_description.as_ref().unwrap().known().unwrap();
    assert_eq!(
        (img.primaries.as_str(), img.transfer.as_str()),
        ("bt2020", "st2084_pq")
    );
    assert_eq!(img.target_primaries.as_deref(), Some("display_p3"));
    assert_eq!(img.reference_luminance_nits, Some(203.0));
    assert_eq!(img.target_max_luminance_nits, Some(1000.0));
    assert_eq!(
        comp(&eps[0])
            .caps
            .modes
            .as_ref()
            .unwrap()
            .known()
            .unwrap()
            .len(),
        2
    );
    assert!(eps[0].correlation.iter().any(
        |c| matches!(&c.id, CorrelationId::WlOutput { name } if name == "HDMI-A-1")
            && c.confidence == Confidence::Measured
    ));
}

#[test]
fn kwin_without_capabilities_reports_no_vrr_or_range() {
    let mut f = kwin_hdr();
    if let Some(Read::Ok(w)) = &mut f.wayland {
        let d = &mut w.kde_devices.as_mut().unwrap()[0];
        d.capabilities = Some(0);
        d.hdr = Some(false);
        w.outputs[0].image_description = None;
    }
    let mut eps = vec![ep("card1-HDMI-A-1")];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    let d = detail(&eps[0]);
    assert!(d.vrr_policy.is_none() && d.rgb_range.is_none());
    assert!(
        d.sdr_brightness_nits.is_none(),
        "SDR brightness only matters with HDR on"
    );
    assert_eq!(
        d.hdr.as_ref().unwrap().known().unwrap().eotfs,
        [Eotf::TraditionalSdr]
    );
}

fn mutter(color_mode: u64, in_logical: bool) -> CompositorFacts {
    let mp = |k: &str, v: serde_json::Value| (k.to_string(), v);
    CompositorFacts {
        wayland: Some(Read::Ok(WaylandFacts {
            globals: globals(&["wl_output", "gtk_shell1"]),
            ..WaylandFacts::default()
        })),
        mutter: Some(Read::Ok(MutterFacts {
            serial: 3,
            monitors: vec![MutterMonitor {
                connector: "HDMI-1".into(),
                vendor: "HEC".into(),
                product: "HISENSE".into(),
                serial: "0x01234567".into(),
                modes: vec![
                    MutterMode {
                        id: "3840x2160@60.000".into(),
                        width: 3840,
                        height: 2160,
                        refresh: 60.0,
                        preferred_scale: 2.0,
                        properties: [
                            mp("is-current", true.into()),
                            mp("is-preferred", true.into()),
                            mp("refresh-rate-mode", "variable".into()),
                        ]
                        .into_iter()
                        .collect(),
                    },
                    MutterMode {
                        id: "3840x2160@23.976".into(),
                        width: 3840,
                        height: 2160,
                        refresh: 23.976,
                        preferred_scale: 2.0,
                        properties: Default::default(),
                    },
                ],
                properties: [
                    mp("color-mode", color_mode.into()),
                    mp("supported-color-modes", serde_json::json!([0, 1])),
                ]
                .into_iter()
                .collect(),
            }],
            logical_monitors: if in_logical {
                vec![MutterLogical {
                    scale: 1.5,
                    transform: 0,
                    primary: true,
                    monitors: vec!["HDMI-1".into()],
                }]
            } else {
                vec![]
            },
            properties: Default::default(),
        })),
        ..CompositorFacts::default()
    }
}

#[test]
fn mutter_bt2100_and_names() {
    let mut eps = vec![ep("card1-HDMI-A-1")];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(mutter(1, true)),
        &ProbeScope::everything(),
        0,
    );
    let d = detail(&eps[0]);
    assert_eq!(d.compositor, CompositorKind::Mutter);
    assert_eq!(d.output_name.known().map(String::as_str), Some("HDMI-1"));
    assert_eq!(d.hdr_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(d.wide_color_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(d.scale.as_ref().unwrap().known(), Some(&1.5));
    assert_eq!(
        d.vrr_policy.as_ref().unwrap().known(),
        Some(&VrrPolicy::Always)
    );
    assert_eq!(
        d.mode.as_ref().unwrap().known().unwrap().refresh_millihz,
        60_000
    );
    let modes = comp(&eps[0]).caps.modes.as_ref().unwrap().known().unwrap();
    assert_eq!(modes[1].refresh_millihz, 23_976);
    assert!(
        d.hdr
            .as_ref()
            .unwrap()
            .known()
            .unwrap()
            .eotfs
            .contains(&Eotf::Pq)
    );
    // Mutter's HDMI-1 is DRM's HDMI-A-1: joined, but only inferred.
    let key = eps[0]
        .correlation
        .iter()
        .find(|c| matches!(c.id, CorrelationId::MutterMonitor { .. }))
        .unwrap();
    assert_eq!(key.confidence, Confidence::Inferred);
    let CorrelationId::MutterMonitor { serial, .. } = &key.id else {
        unreachable!()
    };
    assert_eq!(serial.value, SensitiveValue::Clear("0x01234567".into()));
}

#[test]
fn mutter_monitor_off_and_sdr() {
    let mut eps = vec![ep("card1-HDMI-A-1")];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(mutter(0, false)),
        &ProbeScope::everything(),
        0,
    );
    let d = detail(&eps[0]);
    assert_eq!(d.enabled.known(), Some(&false));
    assert!(
        d.mode.is_none() && d.scale.is_none(),
        "no mode or scale for a monitor that is off"
    );
    assert_eq!(d.hdr_enabled.as_ref().unwrap().known(), Some(&false));
}

fn wlroots(image: Option<Read<ImageInfo>>) -> CompositorFacts {
    CompositorFacts {
        wayland: Some(Read::Ok(WaylandFacts {
            globals: globals(&["wl_output", "zwlr_output_manager_v1", "zwlr_layer_shell_v1"]),
            outputs: vec![WlOutput {
                version: 4,
                name: Some("DP-1".into()),
                mode: Some(mode(2560, 1440, 143_998)),
                scale: Some(1),
                image_description: image,
                ..WlOutput::default()
            }],
            wlr_heads: Some(vec![
                WlrHead {
                    name: "DP-1".into(),
                    enabled: true,
                    modes: vec![mode(2560, 1440, 143_998), mode(2560, 1440, 59_951)],
                    current_mode: Some(0),
                    scale: Some(1.25),
                    adaptive_sync: Some(true),
                    ..WlrHead::default()
                },
                WlrHead {
                    name: "HDMI-A-1".into(),
                    enabled: false,
                    modes: vec![mode(1920, 1080, 60_000)],
                    current_mode: Some(0),
                    scale: Some(1.0),
                    ..WlrHead::default()
                },
            ]),
            ..WaylandFacts::default()
        })),
        ..CompositorFacts::default()
    }
}

#[test]
fn wlroots_without_color_management_is_partial() {
    let mut eps = vec![ep("card0-DP-1"), ep("card0-HDMI-A-1"), ep("card0-DP-2")];
    let a = attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(wlroots(None)),
        &ProbeScope::everything(),
        0,
    );
    assert_eq!(
        a,
        Availability::Read,
        "the sources were read; the output layer is partial"
    );
    let d = detail(&eps[0]);
    assert_eq!(d.compositor, CompositorKind::Wlroots);
    assert_eq!(comp(&eps[0]).availability, Availability::Partial);
    assert!(matches!(
        d.hdr_enabled.as_ref().unwrap(),
        Value::Unknown {
            reason: UnknownReason::NotExposed,
            ..
        }
    ));
    assert_eq!(d.scale.as_ref().unwrap().known(), Some(&1.25));
    assert_eq!(
        d.vrr_policy.as_ref().unwrap().known(),
        Some(&VrrPolicy::Always)
    );
    // A disabled head is still listed, without mode or scale.
    let off = detail(&eps[1]);
    assert_eq!(off.enabled.known(), Some(&false));
    assert!(off.mode.is_none() && off.scale.is_none());
    // A connector the compositor doesn't list.
    assert!(matches!(
        comp(&eps[2]).availability,
        Availability::Unknown {
            reason: UnknownReason::NotExposed,
            ..
        }
    ));
}

#[test]
fn wlroots_with_a_pq_image_description_infers_hdr() {
    let mut eps = vec![ep("card0-DP-1"), ep("card0-HDMI-A-1")];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(wlroots(Some(Read::Ok(pq_image())))),
        &ProbeScope::everything(),
        0,
    );
    let d = detail(&eps[0]);
    assert_eq!(d.hdr_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(
        conf(d.hdr_enabled.as_ref().unwrap()),
        Some(Confidence::Inferred)
    );
    assert_eq!(
        d.sdr_brightness_nits.as_ref().unwrap().known(),
        Some(&203.0)
    );
    assert_eq!(comp(&eps[0]).availability, Availability::Read);
    // An image description the compositor refused is unknown with its message.
    let mut eps = vec![ep("card0-DP-1")];
    let failed = Read::err(
        UnknownReason::NotExposed,
        "image description failed (cause 1): x",
    );
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(wlroots(Some(failed))),
        &ProbeScope::everything(),
        0,
    );
    assert!(matches!(
        detail(&eps[0]).image_description.as_ref().unwrap(),
        Value::Unknown { .. }
    ));
}

fn randr(names: &[(&str, Option<&[u8]>)]) -> CompositorFacts {
    CompositorFacts {
        randr: Some(Read::Ok(RandrFacts {
            vendor: Some("The X.Org Foundation".into()),
            release: Some(12_101_016),
            version: [1, 6],
            outputs: names
                .iter()
                .map(|(n, e)| RandrOutput {
                    name: n.to_string(),
                    connection: "connected".into(),
                    modes: vec![RandrMode {
                        width: 1920,
                        height: 1080,
                        refresh_mhz: 60_000,
                        interlaced: false,
                    }],
                    preferred: 1,
                    current: Some(RandrMode {
                        width: 1920,
                        height: 1080,
                        refresh_mhz: 60_000,
                        interlaced: false,
                    }),
                    properties: e
                        .map(|e| {
                            [
                                ("EDID".to_string(), to_hex(e)),
                                ("max bpc".to_string(), "10".to_string()),
                                ("Broadcast RGB".to_string(), "Full".to_string()),
                                ("Colorspace".to_string(), "BT2020_RGB".to_string()),
                            ]
                            .into_iter()
                            .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect(),
        })),
        ..CompositorFacts::default()
    }
}

#[test]
fn randr_joins_amdgpu_names_by_edid_and_keeps_strangers() {
    let a = edid("workstation-card1-DP-2.bin");
    let b = edid("lab-gnome-card0-Virtual-1.bin");
    // amdgpu's X driver numbers from 0 and names DP outputs DisplayPort-N.
    let mut eps = vec![
        with_sink(ep("card1-DP-1"), &b),
        with_sink(ep("card1-DP-2"), &a),
    ];
    let f = randr(&[("DisplayPort-1", Some(&a)), ("HDMI-A-0", None)]);
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    let d = detail(&eps[1]);
    assert_eq!(d.compositor, CompositorKind::Xorg);
    assert_eq!(
        d.output_name.known().map(String::as_str),
        Some("DisplayPort-1")
    );
    assert_eq!(d.max_bpc.as_ref().unwrap().known(), Some(&10));
    assert_eq!(
        d.rgb_range.as_ref().unwrap().known().map(String::as_str),
        Some("full")
    );
    assert_eq!(d.hdr_enabled.as_ref().unwrap().known(), Some(&false));
    assert_eq!(d.wide_color_enabled.as_ref().unwrap().known(), Some(&true));
    assert_eq!(eps[1].correlation[0].confidence, Confidence::Inferred);
    // card1-DP-1 has a different sink: not joined, and not listed by the compositor.
    assert!(matches!(
        comp(&eps[0]).availability,
        Availability::Unknown { .. }
    ));
    // HDMI-A-0 matches no connector and has no EDID: an endpoint of its own.
    let own = eps
        .iter()
        .find(|e| e.id.0 == "compositor-HDMI-A-0")
        .unwrap();
    assert_eq!(own.class, EndpointClass::External);
    assert_eq!(own.connector.known(), Some(&ConnectorType::Hdmi));
    assert_eq!(comp(own).availability, Availability::Read);
}

#[test]
fn ties_between_cards_are_settled_by_the_sink() {
    let a = edid("workstation-card1-DP-2.bin");
    let b = edid("workstation-card1-DP-3.bin");
    let mut eps = vec![
        with_sink(ep("card0-DP-1"), &b),
        with_sink(ep("card1-DP-1"), &a),
    ];
    let f = randr(&[("DP-1", Some(&a))]);
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    assert!(matches!(
        comp(&eps[0]).availability,
        Availability::Unknown { .. }
    ));
    assert_eq!(comp(&eps[1]).availability, Availability::Read);
}

#[test]
fn scope_limits_what_is_reported() {
    use displaycaps_model::FieldGroup as G;
    let scope = ProbeScope::nothing()
        .with_layer(LayerKind::Compositor)
        .with_field(G::Hdr);
    let mut eps = vec![ep("card1-HDMI-A-1")];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(kwin_hdr()),
        &scope,
        0,
    );
    let l = comp(&eps[0]);
    assert!(
        l.detail.is_none(),
        "compositor detail needs the compositor field group"
    );
    assert!(l.caps.modes.is_none());
    assert!(l.caps.hdr.is_some());
    assert!(
        eps[0].correlation.is_empty(),
        "keys need the identity field group"
    );
    // An output outside the endpoint scope is not reported.
    let scope = ProbeScope::everything().with_endpoint(EndpointId("card9-DP-9".into()));
    let mut eps = vec![];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(kwin_hdr()),
        &scope,
        0,
    );
    assert!(eps.is_empty());
}

#[test]
fn unreadable_compositor_is_unknown_everywhere() {
    let mut eps = vec![ep("card1-HDMI-A-1")];
    let r = CompRead::Unreadable(UnknownReason::NoSession, "no session".into());
    let a = attach(
        &mut eps,
        &Default::default(),
        &r,
        &ProbeScope::everything(),
        0,
    );
    let unknown = |a: &Availability| {
        matches!(
            a,
            Availability::Unknown {
                reason: UnknownReason::NoSession,
                ..
            }
        )
    };
    assert!(unknown(&a) && unknown(&comp(&eps[0]).availability));
    // A source that failed while another was read: partial.
    let mut f = mutter(0, true);
    f.mutter = Some(Read::err(UnknownReason::NotGranted, "AccessDenied"));
    assert_eq!(availability(&CompRead::Facts(f)), Availability::Partial);
}

#[test]
fn identities_and_protocols() {
    let kind = |g: &[&str]| {
        identity(&CompositorFacts {
            wayland: Some(Read::Ok(WaylandFacts {
                globals: globals(g),
                ..WaylandFacts::default()
            })),
            ..CompositorFacts::default()
        })
        .unwrap()
        .0
        .kind
    };
    assert_eq!(kind(&["kde_output_device_v2"]), CompositorKind::Kwin);
    assert_eq!(kind(&["gtk_shell1"]), CompositorKind::Mutter);
    assert_eq!(kind(&["zwlr_layer_shell_v1"]), CompositorKind::Wlroots);
    assert_eq!(
        kind(&["hyprland_ctm_control_manager_v1", "zwlr_layer_shell_v1"]),
        CompositorKind::Hyprland
    );
    assert_eq!(
        kind(&["gamescope_swapchain_factory_v2"]),
        CompositorKind::Gamescope
    );
    assert_eq!(kind(&["weston_capture_v1"]), CompositorKind::Weston);
    assert_eq!(kind(&["wl_compositor"]), CompositorKind::Other);
    let (info, protocols) = identity(&randr(&[])).unwrap();
    assert_eq!(
        (info.kind, info.version.as_deref()),
        (CompositorKind::Xorg, Some("1.21.1"))
    );
    assert_eq!(protocols, ["RANDR 1.6"]);
    let (_, protocols) = identity(&kwin_hdr()).unwrap();
    assert_eq!(
        protocols,
        [
            "wl_output@1",
            "wp_color_manager_v1@1",
            "kde_output_device_v2@1"
        ]
    );
    // GNOME on X11: Mutter owns the configuration.
    let mut f = randr(&[]);
    f.mutter = mutter(0, true).mutter;
    assert_eq!(identity(&f).unwrap().0.kind, CompositorKind::Mutter);
}

#[test]
fn primaries_and_names() {
    assert_eq!(
        name_primaries(&[
            708_100, 291_900, 170_000, 797_000, 131_000, 46_000, 312_700, 329_000
        ]),
        "bt2020"
    );
    assert_eq!(name_primaries(&[1, 2, 3, 4, 5, 6, 7, 8]), "custom");
    assert_eq!(drm_style("HDMI-1"), "HDMI-A-1");
    assert_eq!(drm_style("HDMI-A-1"), "HDMI-A-1");
    assert_eq!(drm_style("DP-1"), "DP-1");
    assert_eq!(connector_of(&EndpointId("card10-eDP-1".into())), "eDP-1");
    assert_eq!(class_of("eDP-1").0, EndpointClass::Internal);
    assert_eq!(class_of("HEADLESS-1").0, EndpointClass::Virtual);
    assert_eq!(xorg_version(12_101_016), "1.21.1");
}

#[test]
fn fixture_facts_replay() {
    let dir = format!(
        "{}/../../tests/fixtures/lab-plasma",
        env!("CARGO_MANIFEST_DIR")
    );
    let root = SystemRoot::fixture(dir).unwrap();
    let Access::Fixture(CompRead::Facts(f)) = access(&root, &Env::Absent, Want::Outputs) else {
        panic!("the lab fixture has compositor facts")
    };
    assert_eq!(identity(&f).unwrap().0.kind, CompositorKind::Kwin);
}

#[test]
fn zero_based_x_names_join_by_edid_even_without_a_sink_record() {
    let a = edid("workstation-card1-DP-2.bin");
    let b = edid("workstation-card1-DP-3.bin");
    // amdgpu's X driver: HDMI-A-0 is DRM's HDMI-A-1, HDMI-A-1 is DRM's HDMI-A-2. The sink layer
    // isn't in scope, so only the connectors' EDIDs (read for the join) tell them apart.
    let mut eps = vec![ep("card0-HDMI-A-1"), ep("card0-HDMI-A-2")];
    let edids: std::collections::BTreeMap<String, Vec<u8>> = [
        ("card0-HDMI-A-1".to_string(), a.clone()),
        ("card0-HDMI-A-2".to_string(), b.clone()),
    ]
    .into_iter()
    .collect();
    let f = randr(&[("HDMI-A-0", Some(&a)), ("HDMI-A-1", Some(&b))]);
    attach(
        &mut eps,
        &edids,
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    assert_eq!(eps.len(), 2, "no extra endpoints");
    let name = |e: &Endpoint| detail(e).output_name.known().cloned();
    assert_eq!(name(&eps[0]).as_deref(), Some("HDMI-A-0"));
    assert_eq!(name(&eps[1]).as_deref(), Some("HDMI-A-1"));
    assert!(
        eps.iter()
            .all(|e| e.correlation[0].confidence == Confidence::Inferred)
    );
}

#[test]
fn a_second_output_for_a_taken_connector_is_not_a_new_endpoint() {
    let a = edid("workstation-card1-DP-2.bin");
    let mut eps = vec![ep("card0-HDMI-A-1")];
    let edids = [("card0-HDMI-A-1".to_string(), a.clone())]
        .into_iter()
        .collect();
    let f = randr(&[("HDMI-A-1", Some(&a)), ("HDMI-1-1", Some(&a))]);
    attach(
        &mut eps,
        &edids,
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    assert_eq!(eps.len(), 1);
    assert_eq!(
        eps[0]
            .layers
            .iter()
            .filter(|l| l.layer == LayerKind::Compositor)
            .count(),
        1
    );
}

#[test]
fn xwayland_hides_the_hosts_hdr_state() {
    let f = randr(&[("XWAYLAND0", None)]);
    let mut eps = vec![];
    attach(
        &mut eps,
        &Default::default(),
        &CompRead::Facts(f.clone()),
        &ProbeScope::everything(),
        0,
    );
    let e = &eps[0];
    assert_eq!(e.id.0, "compositor-XWAYLAND0");
    assert_eq!(e.class, EndpointClass::Virtual);
    assert!(matches!(
        detail(e).hdr_enabled.as_ref().unwrap(),
        Value::Unknown {
            reason: UnknownReason::NotGranted,
            ..
        }
    ));
    assert_eq!(identity(&f).unwrap().0.kind, CompositorKind::Other);
}
