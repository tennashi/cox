//! What a device's screen shows as the apps lay it out, as `uiautomator
//! dump` writes it: each element, with its text, what it is, and where it
//! is in the display's own pixels; told to the agent in the pixels of the
//! picture it sees, and found by its text or resource id to tap or type
//! into.

use std::fmt::Write as _;

/// A stretch of the display, in its own pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Rect {
    /// The middle of the stretch.
    #[must_use]
    pub fn center(self) -> (u32, u32) {
        (
            self.left + (self.right.saturating_sub(self.left)) / 2,
            self.top + (self.bottom.saturating_sub(self.top)) / 2,
        )
    }

    fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// An element of the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// How deep it is among the elements, the windows' own at 0.
    pub depth: usize,
    /// Its class, as `android.widget.Button`.
    pub class: String,
    pub text: String,
    /// What it says to accessibility, as an icon's button does.
    pub desc: String,
    /// Its resource id, as `com.example:id/save`.
    pub id: String,
    pub bounds: Rect,
    /// What it is or does, in the order the agent is told them.
    pub flags: Vec<Flag>,
}

/// What an element is or does, as the agent is told it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Clickable,
    LongClickable,
    Scrollable,
    Focused,
    Checked,
    Unchecked,
    Disabled,
}

impl Flag {
    /// The flags as a dump's attributes say them, by `attribute`, which
    /// answers whether one is `"true"`, and the agent is told them.
    const TOLD: [(Self, &str, bool, &str); 7] = [
        (Self::Clickable, "clickable", true, "clickable"),
        (
            Self::LongClickable,
            "long-clickable",
            true,
            "long-clickable",
        ),
        (Self::Scrollable, "scrollable", true, "scrollable"),
        (Self::Focused, "focused", true, "focused"),
        (Self::Checked, "checked", true, "checked"),
        (Self::Unchecked, "checked", false, "unchecked"),
        (Self::Disabled, "enabled", false, "disabled"),
    ];

    fn name(self) -> &'static str {
        Self::TOLD
            .iter()
            .find(|(flag, ..)| *flag == self)
            .map_or("", |(.., name)| name)
    }

    /// Whether an element with it is acted on.
    fn is_acted_on(self) -> bool {
        matches!(
            self,
            Self::Clickable | Self::LongClickable | Self::Scrollable | Self::Focused
        )
    }
}

impl Node {
    /// Its resource id after the package's, as `save`.
    #[must_use]
    pub fn short_id(&self) -> &str {
        self.id.rsplit_once(":id/").map_or(&self.id, |(_, id)| id)
    }

    /// Whether the agent is told of it: one that says something, or is
    /// acted on; the layouts holding them are left out.
    fn is_told(&self) -> bool {
        !self.bounds.is_empty()
            && (!self.text.is_empty()
                || !self.desc.is_empty()
                || !self.id.is_empty()
                || self.flags.iter().any(|flag| flag.is_acted_on()))
    }
}

/// The elements of a dump, in order, each before those it holds.
///
/// # Errors
///
/// When `xml` is not a dump's.
pub fn parse(xml: &str) -> Result<Vec<Node>, String> {
    let document = roxmltree::Document::parse(xml.trim())
        .map_err(|error| format!("the screen's elements are not readable: {error}"))?;
    let mut nodes = Vec::new();
    for element in document
        .descendants()
        .filter(|node| node.has_tag_name("node"))
    {
        let depth = element
            .ancestors()
            .filter(|node| node.has_tag_name("node"))
            .count()
            - 1;
        let text = |name: &str| element.attribute(name).unwrap_or_default().to_owned();
        let checkable = element.attribute("checkable") == Some("true");
        let Some(bounds) = element.attribute("bounds").and_then(bounds_of) else {
            continue;
        };
        nodes.push(Node {
            depth,
            class: text("class"),
            text: text("text"),
            desc: text("content-desc"),
            id: text("resource-id"),
            bounds,
            flags: Flag::TOLD
                .iter()
                .filter(|(flag, attribute, is, _)| {
                    let checks = matches!(flag, Flag::Checked | Flag::Unchecked);
                    (checkable || !checks)
                        && element
                            .attribute(*attribute)
                            .is_some_and(|value| (value == "true") == *is)
                })
                .map(|(flag, ..)| *flag)
                .collect(),
        });
    }
    Ok(nodes)
}

/// `[left,top][right,bottom]`, as a dump writes bounds.
fn bounds_of(text: &str) -> Option<Rect> {
    let numbers: Vec<u32> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [left, top, right, bottom] = numbers[..] else {
        return None;
    };
    Some(Rect {
        left,
        top,
        right,
        bottom,
    })
}

/// How the display's own pixels go to those of the picture the agent sees
/// of it, whose sides are the display's, scaled alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    /// The display's size, in its own pixels.
    pub display: (u32, u32),
    /// The picture's size, in its pixels.
    pub picture: (u32, u32),
}

impl Scale {
    /// The picture's pixel where the display's `x` and `y` are.
    #[must_use]
    pub fn to_picture(self, (x, y): (u32, u32)) -> (u32, u32) {
        let along = |at: u32, from: u32, to: u32| {
            let at = u64::from(at) * u64::from(to) + u64::from(from.max(1)) / 2;
            u32::try_from(at / u64::from(from.max(1))).unwrap_or(u32::MAX)
        };
        (
            along(x, self.display.0, self.picture.0),
            along(y, self.display.1, self.picture.1),
        )
    }

    fn rect(self, rect: Rect) -> Rect {
        let (left, top) = self.to_picture((rect.left, rect.top));
        let (right, bottom) = self.to_picture((rect.right, rect.bottom));
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// What the agent asks for an element by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Find {
    /// Its text, or what it says to accessibility: the whole of it, or, if
    /// no element has the whole, a part.
    Text(String),
    /// Its resource id, whole or after the package's, as `save`.
    Id(String),
}

impl Find {
    /// The one element of `nodes` the agent asks for.
    ///
    /// # Errors
    ///
    /// When none is, or more than one, saying which, in the pixels of the
    /// picture `scale` goes to.
    pub fn in_nodes<'a>(&self, nodes: &'a [Node], scale: Scale) -> Result<&'a Node, String> {
        let (found, what): (Vec<&Node>, String) = match self {
            Self::Text(text) => {
                let whole: Vec<&Node> = nodes
                    .iter()
                    .filter(|node| node.text == *text || node.desc == *text)
                    .collect();
                let found = if whole.is_empty() {
                    nodes
                        .iter()
                        .filter(|node| node.text.contains(text) || node.desc.contains(text))
                        .collect()
                } else {
                    whole
                };
                (found, format!("text {text:?}"))
            }
            Self::Id(id) => (
                nodes
                    .iter()
                    .filter(|node| node.id == *id || node.short_id() == id)
                    .collect(),
                format!("resource id {id:?}"),
            ),
        };
        match found[..] {
            [one] => Ok(one),
            [] => Err(format!(
                "No element on the screen has the {what}: android_ui lists them."
            )),
            ref many => {
                let mut told = format!(
                    "{} elements have the {what}; tap one by x and y:",
                    many.len()
                );
                for node in many.iter().take(8) {
                    let _ = write!(told, "\n{}", line(node, scale));
                }
                Err(told)
            }
        }
    }
}

/// The elements the agent is told of, one a line, held ones indented
/// under those holding them, in the pixels of the picture `scale` goes to.
#[must_use]
pub fn told(nodes: &[Node], scale: Scale) -> String {
    let mut told = String::new();
    // The depths of the told elements holding the one now.
    let mut holders: Vec<usize> = Vec::new();
    for node in nodes.iter().filter(|node| node.is_told()) {
        while holders.last().is_some_and(|depth| *depth >= node.depth) {
            holders.pop();
        }
        let _ = writeln!(told, "{}{}", "  ".repeat(holders.len()), line(node, scale));
        holders.push(node.depth);
    }
    told
}

/// One element, as the agent is told of it.
fn line(node: &Node, scale: Scale) -> String {
    let class = node.class.rsplit('.').next().unwrap_or(&node.class);
    let mut line = class.to_owned();
    if !node.text.is_empty() {
        let _ = write!(line, " {:?}", node.text);
    }
    if !node.desc.is_empty() && node.desc != node.text {
        let _ = write!(line, " desc={:?}", node.desc);
    }
    if !node.id.is_empty() {
        let _ = write!(line, " id={}", node.short_id());
    }
    let rect = scale.rect(node.bounds);
    let (x, y) = scale.to_picture(node.bounds.center());
    let _ = write!(
        line,
        " at {x},{y} [{},{} {},{}]",
        rect.left, rect.top, rect.right, rect.bottom
    );
    for flag in &node.flags {
        let _ = write!(line, " {}", flag.name());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = include_str!("../tests/fixtures/ui.xml");

    /// The fixture's display, 1080×2424, seen as a 570×1280 picture.
    const SCALE: Scale = Scale {
        display: (1080, 2424),
        picture: (570, 1280),
    };

    fn nodes() -> Vec<Node> {
        parse(DUMP).unwrap()
    }

    #[test]
    fn a_dump_is_read_with_each_element_and_how_deep_it_is() {
        let nodes = nodes();
        assert_eq!(nodes.len(), 11);
        let save = &nodes[4];
        assert_eq!(
            (save.text.as_str(), save.short_id(), save.depth),
            ("保存", "save", 2)
        );
        assert_eq!(
            save.bounds,
            Rect {
                left: 723,
                top: 83,
                right: 954,
                bottom: 209
            }
        );
        assert_eq!(save.flags, [Flag::Clickable]);
        assert_eq!(nodes[3].text, "Tables & seats");
        assert_eq!(nodes[7].text, "Table 3\n2 guests");
        assert!(nodes[8].flags.contains(&Flag::Focused));
    }

    #[test]
    fn what_is_not_a_dump_is_an_error() {
        assert!(parse("ERROR: could not get idle state.").is_err());
    }

    #[test]
    fn the_displays_pixels_go_to_the_pictures() {
        assert_eq!(SCALE.to_picture((0, 0)), (0, 0));
        assert_eq!(SCALE.to_picture((1080, 2424)), (570, 1280));
        // 838.5 and 146 of the display: the middle of 保存.
        assert_eq!(SCALE.to_picture((838, 146)), (442, 77));
    }

    #[test]
    fn the_agent_is_told_the_elements_that_say_or_do_something_in_the_pictures_pixels() {
        let told = told(&nodes(), SCALE);
        let expected = "\
ViewGroup id=toolbar at 285,77 [0,39 570,116]
  ImageButton desc=\"Cancel\" at 39,77 [0,39 78,116] clickable
  TextView \"Tables & seats\" at 187,77 [83,55 291,100]
  Button \"保存\" id=save at 442,77 [382,44 504,110] clickable
RecyclerView id=tables at 285,586 [0,116 570,1056] scrollable
  LinearLayout at 285,169 [0,116 570,222] clickable long-clickable
    TextView \"Table 3\\n2 guests\" id=name at 195,169 [21,127 369,211]
EditText \"S0\" id=staff_id at 282,1122 [72,1083 492,1162] clickable long-clickable focused
EditText \"••••\" id=password at 282,1217 [72,1178 492,1257] clickable long-clickable
";
        assert_eq!(told, expected);
    }

    #[test]
    fn an_element_is_found_by_its_whole_text_before_a_part() {
        let nodes = nodes();
        let found = Find::Text("保存".to_owned()).in_nodes(&nodes, SCALE);
        assert_eq!(found.map(Node::short_id), Ok("save"));
        let found = Find::Text("Table 3".to_owned()).in_nodes(&nodes, SCALE);
        assert_eq!(found.map(Node::short_id), Ok("name"));
        let found = Find::Text("Cancel".to_owned()).in_nodes(&nodes, SCALE);
        assert_eq!(
            found.map(|node| node.class.as_str()),
            Ok("android.widget.ImageButton")
        );
    }

    #[test]
    fn an_element_is_found_by_its_whole_or_short_resource_id() {
        let nodes = nodes();
        for id in ["password", "com.example.pos:id/password"] {
            let found = Find::Id(id.to_owned()).in_nodes(&nodes, SCALE);
            assert_eq!(found.map(|node| node.text.as_str()), Ok("••••"));
        }
    }

    #[test]
    fn no_element_or_many_are_errors_saying_where_the_many_are() {
        let nodes = nodes();
        assert!(
            Find::Id("nothing".to_owned())
                .in_nodes(&nodes, SCALE)
                .is_err()
        );
        let many = Find::Text("a".to_owned())
            .in_nodes(&nodes, SCALE)
            .unwrap_err();
        assert!(many.starts_with("3 elements have"), "{many}");
        assert!(
            many.contains("TextView \"Tables & seats\" at 187,77"),
            "{many}"
        );
    }
}
