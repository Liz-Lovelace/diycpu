use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type ComponentId = String;
pub type WireNetId = String;
pub type JunctionId = String;

/// The complete project as saved to disk in JSON format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub steps: usize,
    #[serde(default)]
    pub abstractions: Vec<ProjectAbstraction>,
    pub components: Vec<ProjectComponent>,
    #[serde(default)]
    pub wire_nets: Vec<WireNet>,
    /// Free-floating text annotations. Purely visual; the engine ignores them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Pictures placed on the canvas. Purely visual; the engine ignores them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<MediaImage>,
}

/// A text annotation placed on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub pos: [f32; 2],
}

/// A picture placed on the canvas. `file` is a path relative to the
/// project's directory (media files live in its `media/` folder).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaImage {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub pos: [f32; 2],
    /// Displayed size in grid units.
    #[serde(default = "default_image_size")]
    pub size: [f32; 2],
}

fn default_image_size() -> [f32; 2] {
    [6.0, 4.0]
}

/// A component as stored in a project file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectComponent {
    pub id: ComponentId,
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<bool>,
    #[serde(default)]
    pub pos: [f32; 2],
}

/// An abstraction definition within a project, including visual data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAbstraction {
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: HashMap<String, String>,
    /// Width for instances of this abstraction on the canvas, in grid units.
    #[serde(default = "default_width")]
    pub width: f32,
    /// Height for instances of this abstraction on the canvas, in grid units.
    #[serde(default = "default_height")]
    pub height: f32,
    /// The sub-circuit components.
    pub components: Vec<ProjectComponent>,
    /// Wire nets within this abstraction's sub-circuit.
    #[serde(default)]
    pub wire_nets: Vec<WireNet>,
    /// Annotations on this abstraction's sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Pictures on this abstraction's sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<MediaImage>,
    /// Where each interface pin sits inside this abstraction's sheet, in
    /// grid units. Pins without an entry get an automatic rail position.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub pin_positions: HashMap<String, [f32; 2]>,
    /// Where each port sits on the CLOSED rectangle (any edge). Ports
    /// without an entry lay out automatically (inputs left, outputs right).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub port_layout: HashMap<String, PortSpot>,
}

/// A port's spot on the closed rectangle: an edge plus a 0..1 fraction
/// along it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PortSpot {
    pub side: PortSide,
    pub t: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortSide {
    Left,
    Right,
    Top,
    Bottom,
}

fn default_width() -> f32 {
    4.0
}

fn default_height() -> f32 {
    2.0
}

/// Identifies a specific port on a specific component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortRef {
    pub component_id: ComponentId,
    pub port_name: String,
}

/// A junction in a wire net — a positioned routing point that wire segments connect through.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Junction {
    pub id: JunctionId,
    pub pos: [f32; 2],
}

/// One endpoint of a wire segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum Endpoint {
    /// Connected to a component port.
    Port {
        component_id: ComponentId,
        port_name: String,
    },
    /// Connected to a junction by ID.
    Junction { junction_id: JunctionId },
}

/// A single wire segment — a straight line between two endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// A wire net: one output source driving zero or more input sinks,
/// routed through junctions and segments.
///
/// `source` and `sinks` capture the logical connection (what the engine needs).
/// `junctions` and `segments` capture the visual routing (what the GUI needs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireNet {
    pub id: WireNetId,
    /// The output port that drives this net. None if unconnected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PortRef>,
    /// All input ports that this net connects to.
    #[serde(default)]
    pub sinks: Vec<PortRef>,
    /// Free-form junctions for routing.
    #[serde(default)]
    pub junctions: Vec<Junction>,
    /// Segments that define the visual routing.
    #[serde(default)]
    pub segments: Vec<Segment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty_project() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
        };
        let json = serde_json::to_string_pretty(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.steps, 8);
        assert!(parsed.components.is_empty());
    }

    #[test]
    fn round_trip_component() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 1,
            abstractions: vec![],
            components: vec![
                ProjectComponent {
                    id: "clk_1".into(),
                    component_type: "clk".into(),
                    init: None,
                    pos: [0.0, 0.0],
                },
                ProjectComponent {
                    id: "dff_1".into(),
                    component_type: "dff".into(),
                    init: Some(true),
                    pos: [200.0, 100.0],
                },
            ],
            wire_nets: vec![],
        };
        let json = serde_json::to_string(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.components.len(), 2);
        assert_eq!(parsed.components[0].id, "clk_1");
        assert_eq!(parsed.components[0].component_type, "clk");
        assert_eq!(parsed.components[1].init, Some(true));
        assert_eq!(parsed.components[1].pos, [200.0, 100.0]);
    }

    #[test]
    fn round_trip_wire_net() {
        let net = WireNet {
            id: "w1".into(),
            source: Some(PortRef {
                component_id: "clk_1".into(),
                port_name: "out".into(),
            }),
            sinks: vec![PortRef {
                component_id: "dff_1".into(),
                port_name: "clk".into(),
            }],
            junctions: vec![Junction {
                id: "j1".into(),
                pos: [100.0, 50.0],
            }],
            segments: vec![
                Segment {
                    from: Endpoint::Port {
                        component_id: "clk_1".into(),
                        port_name: "out".into(),
                    },
                    to: Endpoint::Junction {
                        junction_id: "j1".into(),
                    },
                },
                Segment {
                    from: Endpoint::Junction {
                        junction_id: "j1".into(),
                    },
                    to: Endpoint::Port {
                        component_id: "dff_1".into(),
                        port_name: "clk".into(),
                    },
                },
            ],
        };
        let json = serde_json::to_string_pretty(&net).unwrap();
        let parsed: WireNet = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, net);
    }

    #[test]
    fn round_trip_abstraction() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![ProjectAbstraction {
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                notes: vec![],
            images: vec![],
                name: "not".into(),
                inputs: vec!["x".into()],
                outputs: {
                    let mut m = HashMap::new();
                    m.insert("out".into(), "gate".into());
                    m
                },
                width: 140.0,
                height: 70.0,
                components: vec![ProjectComponent {
                    id: "gate_1".into(),
                    component_type: "nand".into(),
                    init: None,
                    pos: [50.0, 30.0],
                }],
                wire_nets: vec![WireNet {
                    id: "aw1".into(),
                    source: Some(PortRef {
                        component_id: "INPUT:x".into(),
                        port_name: "x".into(),
                    }),
                    sinks: vec![
                        PortRef {
                            component_id: "gate_1".into(),
                            port_name: "a".into(),
                        },
                        PortRef {
                            component_id: "gate_1".into(),
                            port_name: "b".into(),
                        },
                    ],
                    junctions: vec![],
                    segments: vec![],
                }],
            }],
            components: vec![],
            wire_nets: vec![],
        };
        let json = serde_json::to_string_pretty(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.abstractions.len(), 1);
        assert_eq!(parsed.abstractions[0].name, "not");
        assert_eq!(parsed.abstractions[0].width, 140.0);
        assert_eq!(parsed.abstractions[0].wire_nets[0].sinks.len(), 2);
    }

    #[test]
    fn defaults_on_missing_optional_fields() {
        let json = r#"{
            "name": "minimal",
            "steps": 5,
            "components": [
                {"id": "c1", "type": "clk"}
            ]
        }"#;
        let project: Project = serde_json::from_str(json).unwrap();
        assert_eq!(project.name, "minimal");
        assert!(project.abstractions.is_empty());
        assert!(project.wire_nets.is_empty());
        assert_eq!(project.components[0].pos, [0.0, 0.0]);
        assert_eq!(project.components[0].init, None);
    }

    #[test]
    fn round_trip_images() {
        let json = r#"{
            "name": "pics",
            "steps": 4,
            "components": [],
            "images": [
                {"id": "img_1", "file": "media/cat.gif", "pos": [2.0, 3.0], "size": [8.0, 5.0]},
                {"id": "img_2", "file": "media/logo.png"}
            ]
        }"#;
        let project: Project = serde_json::from_str(json).unwrap();
        assert_eq!(project.images.len(), 2);
        assert_eq!(project.images[0].file, "media/cat.gif");
        assert_eq!(project.images[1].size, [6.0, 4.0]); // default size
        let back = serde_json::to_string(&project).unwrap();
        let again: Project = serde_json::from_str(&back).unwrap();
        assert_eq!(again.images, project.images);
    }

    #[test]
    fn empty_images_are_not_serialized() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "t".into(),
            steps: 1,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
        };
        assert!(!serde_json::to_string(&project).unwrap().contains("images"));
    }

    #[test]
    fn wire_net_with_no_source() {
        let net = WireNet {
            id: "orphan".into(),
            source: None,
            sinks: vec![],
            junctions: vec![Junction {
                id: "j1".into(),
                pos: [50.0, 50.0],
            }],
            segments: vec![],
        };
        let json = serde_json::to_string(&net).unwrap();
        let parsed: WireNet = serde_json::from_str(&json).unwrap();
        assert!(parsed.source.is_none());
        assert!(parsed.sinks.is_empty());
    }

    #[test]
    fn direct_port_to_port_segment() {
        let net = WireNet {
            id: "direct".into(),
            source: Some(PortRef {
                component_id: "c1".into(),
                port_name: "out".into(),
            }),
            sinks: vec![PortRef {
                component_id: "c2".into(),
                port_name: "a".into(),
            }],
            junctions: vec![],
            segments: vec![Segment {
                from: Endpoint::Port {
                    component_id: "c1".into(),
                    port_name: "out".into(),
                },
                to: Endpoint::Port {
                    component_id: "c2".into(),
                    port_name: "a".into(),
                },
            }],
        };
        let json = serde_json::to_string(&net).unwrap();
        let parsed: WireNet = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.segments.len(), 1);
        assert_eq!(parsed.junctions.len(), 0);
    }
}
