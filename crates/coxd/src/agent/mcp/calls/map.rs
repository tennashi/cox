//! `show_map`: shows the human a map of the project on the stage, opens
//! it where the agent talks of, and tells the agent what is there, how
//! the indexes of the code stand, and the drift, where the map's
//! vocabulary and the code disagree, for the agent to talk it through
//! with the human (see [`crate::map`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use cox_editor::map::generated::{
    CallKind, Function, Generated, Level, Line, Slice, Top, Type, Use,
};
use cox_editor::map::vocabulary::EVERYTHING;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::ask;
use crate::agent::mcp::{ChatRequest, ToolRequest};
use crate::map::{Changed, IndexState, Indexed, State, Told};

pub(super) const SHOW_MAP: &str = "show_map";

/// How a map's vocabulary is written, as the agent is told it when it is
/// to write one: the form the Map page draws, not what to put in it,
/// which is the project's to decide.
pub(super) const MAP_FORMAT: &str = "\
A project's map is a file in `maps` in the project's folder, named after the map, as \
`maps/map.json` for the map `map`; or in `maps` in the folder of one of the project's \
repositories, to keep it with the repository's code, as `directories/app/maps/map.json` for \
the map `app/map`, whose `files` are from the repository's folder. A chat working in a checkout \
of the repository has it as `map`. A project may keep several. The file is the map's \
vocabulary: what the code cannot say. Cox reads the rest from an index of the code of each \
commit, which it makes itself, and marks drift where the two disagree. The map is told from \
whoever starts the project working, its drivers, which its top names: it is shown for one \
driver at a time, what that driver sets working. The vocabulary is a JSON object:\n\
- `says`: one line on what the map shows, as against the project's other maps.\n\
- `nodes`: the boxes, and the people and systems around them, by id, each `{\"name\", \"icon\", \
\"kind\", \"code\", \"many\", \"claims\", \"files\", \"open\", \"lines\"}`. `kind` is `part`, \
the default, or `outside`. `icon` is one of person, phone, agent, folder, globe, android, apps, \
run, download, gear, box, window, layers, cycle, plug, store, braces, terminal, branch, net, \
search, doc, server, clock, chat, check, socket. `code` is what it is to the code, drawn under \
its name: its first claim unless given. `many: true` says there are several of it, as one per \
chat. `claims` are the modules it owns, as the index names them: the package, then the \
namespaces, joined by `/`, as `app/server` or `httpx._client`; a module belongs to the box with \
the longest claim it is in. `files` are the files or folders it owns, from the folder that \
holds the map's `maps`, the project's or its repository's, for code whose smallest module \
holds several files, as a package of Go does; they win over `claims`. `open: false` keeps a \
box that owns code closed. `lines: \"hand\"` takes its lines from the vocabulary rather than \
from the code.\n\
- `top`: `{\"center\", \"drivers\", \"left\", \"right\", \"lines\"}`: the node of the \
project in the middle, which opens on the level inside it; the drivers, `outside` nodes that \
start the project working, as a person by what they use it for, a system that calls it, or a \
clock that sets it off; the other nodes around it, in a column on its left, under the \
drivers, and on its right, from the top; and the lines between them. What a driver sets \
working is what Cox reaches from it: along the lines you write, each from its `from`, through \
the people and systems on the way, into a box, to the parts the level inside draws the line on \
to, or else to all of the box; then, from the types the lines come in at, along the code, the \
way it calls and names; and along a line you wrote from a box that owns code once the code \
has come into the box. A line from a box that opens on boxes is followed from the part the \
level inside draws it on from, or else only where a driver comes in at all of the box: so \
draw each line on, level by level, from the part that starts it. Where it comes to a type another driver's lines come in at, it shows the type and goes \
no further: from there on is the other's. A map with no `drivers` is shown whole.\n\
- `levels`: the levels a person places, by the node each opens, in the coordinates of the \
picture, `y` down: `{\"box\": [x, y, width, height], \"outer\": {id: [side, along]}, \"at\": \
{id: [x, y]}, \"frames\": {id: [x, y, width, height]}, \"parts\": {id: [x, y]}, \"lines\"}`. \
`box` is where the node opened sits; `outer` places the top's nodes around a level the top \
opens on, by the side of `box` (`left`, `right`, `top` or `bottom`) and how far along it; `at` \
places the nodes of the levels above around a level further down, those the code's lines go \
on to: leave it out, and Cox places around `box` every box above that the code of the parts \
meets, as it does any end of the level's lines placed nowhere in it; `frames` are the edges of \
the nodes above that it is inside of; `parts` are the boxes inside it, each 216 by 56. Inside a \
box that owns code, Cox lays out its types. A box with too many types to read on one level \
owns no code itself, and opens on a level of boxes that own it; drawn closed, it stands for \
them, its lines for theirs.\n\
- A line is `{\"from\", \"to\", \"both\", \"sides\", \"stops\"}`: `from` is the end that \
starts it, as the one that asks, and what a driver sets working follows it from there only; \
`both: true` draws it both ways, as a request and its answer; `sides` the sides of `from` and \
of `to` it leaves and meets, unless the nearest; `stops: true` ends what a driver sets working \
at `to`, which is shown and not gone into, as the app a build task starts. Write only the lines no code backs, as between processes, or to the \
people and systems around: those between boxes that own code, or hold boxes that do, come from \
the code, and writing one is a mistake the map reports. A box opened keeps the lines it has \
closed: in the level inside it, draw one on from the same node to the part it reaches, or it \
goes to the box's edge. Or write the end at a box as a rule, `{\"box\", \"implements\"}` or \
`{\"box\", \"uses\"}`, as `{\"from\": \"client\", \"to\": {\"box\": \"server\", \
\"implements\": \"RequestHandler\"}}`, and the line goes on, inside the box, to the types that \
implement the trait or interface, or subclass the class, or whose code names the type or \
function: the boxes that hold them, then the types. Write the name as declared, not as \
imported under another, with as much of its module before it as tells it from others, as \
`tui/Renderer` or `hono-base.ts/Hono`; rust-analyzer's `impl` blocks name a trait by its last \
part alone. It sees only indexed code, not what a build or a macro leaves out, and reaches \
types only, not free functions. A rule that picks nothing is drift.\n\
- `agreed`: the drift talked through and left as it is, each `{\"id\", \"why\"}`, by the id \
the map gives it.\n\
Change the vocabulary as you move the code it claims.";

/// The prompt that has the agent make a map with the human, as the
/// agent's client offers it, as a slash command; its text is in
/// `make_map.md`, with `{name}` and `{drivers}` for its arguments.
pub(super) const MAKE_MAP: &str = "make_map";
const MAKE_MAP_TEXT: &str = include_str!("map/make_map.md");
const MAP_NAMED: &str = "map";
const ANY_DRIVERS: &str = "for us to settle together";

/// The prompt `make_map`, as `prompts/list` lists it.
pub(super) fn prompt() -> Value {
    json!({
        "name": MAKE_MAP,
        "title": "Make a map of the project",
        "description": "Has the agent survey the project, settle with you who starts it \
            working, write a map's vocabulary, and work its drift down with you.",
        "arguments": [
            {
                "name": "name",
                "description": "The map's name, its file in `maps` without `.json`; `map` \
                    when not given.",
            },
            {
                "name": "drivers",
                "description": "Who and what starts the project working, as you know it \
                    already, as \"people using it, people building it, the nightly job\"; \
                    settled with you when not given.",
            },
        ],
    })
}

/// The prompt `make_map`, as `prompts/get` answers it, with its
/// `arguments` filled in, and the form of a vocabulary after it.
pub(super) fn make_map(arguments: &Value) -> Result<Value, String> {
    let argument = |name: &str, otherwise: &'static str| match arguments.get(name) {
        None | Some(Value::Null) => Ok(otherwise),
        Some(Value::String(given)) if given.trim().is_empty() => Ok(otherwise),
        Some(Value::String(given)) => Ok(given.trim()),
        Some(_) => Err(format!("{name} is to be a string")),
    };
    let name = argument("name", MAP_NAMED)?;
    let drivers = argument("drivers", ANY_DRIVERS)?;
    let text = MAKE_MAP_TEXT
        .replace("{name}", name)
        .replace("{drivers}", drivers);
    Ok(json!({
        "description": format!("Make the map `{name}` of the project"),
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": format!("{text}\n{MAP_FORMAT}") },
        }],
    }))
}

/// How many places in the code an item of drift is told with, at most.
const SITES: usize = 3;

/// How many items of drift the agent is told of, at most, those on what
/// the map shows first, and how many agreed ones: the Map tab lists all.
const DRIFT_TOLD: usize = 15;
const AGREED_TOLD: usize = 10;

/// How many of a type's fields, and of its functions, the agent is told
/// of, at most; and of a function's callers, and of what it calls.
const MEMBERS_TOLD: usize = 40;
const CALLERS_TOLD: usize = 20;
const CALLS_TOLD: usize = 30;

/// The tool that shows the map of the project, as the agent learns of it.
pub(super) fn tool() -> Value {
    json!(
    {
        "name": SHOW_MAP,
        "description": "Shows the human a map of the project on a tab of the stage beside \
            the conversation: its parts as boxes, from the few things it is among at the top, \
            through what is inside each, down to the types of the code each box owns, with \
            their fields and functions, where each is used and what each calls, and what \
            changed since the last commit on the boxes it falls in. A person keeps the map's \
            vocabulary, the boxes, where they sit and the code each owns, in `maps` in the \
            project's folder, or in that of one of its repositories; Cox reads the rest from \
            an index of the code of each commit, and marks drift where the vocabulary and the \
            code disagree. Use it to show the human where what you talk of is in the project, \
            before you talk it through or show the diff: open it at the box or the type you \
            talk of. Answers what the map shows there, how the indexes of the code stand, and \
            the drift, each by its id, to talk through with the human: to fix the vocabulary, \
            change the code, or agree to leave it. A map whose top names drivers, who \
            starts the project working, is shown for one of them at a time, what that \
            driver sets working: name it as `driver`. Write a vocabulary when the project has \
            none, or when the human wants another viewpoint of it, deciding with the human \
            what it is to show; shown with none, or with one that does not read, the tool says \
            how it is written.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "map": {
                    "type": "string",
                    "description": "Which map to open, by its name, its vocabulary's file \
                        in `maps` without `.json`, after its repository's name and `/` when a \
                        repository of the project keeps it, as `app/use`; the project's first \
                        when not given.",
                },
                "driver": {
                    "type": "string",
                    "description": "Whose slice of the map to open, when the map's top names \
                        drivers, the people and systems that start the project working: a \
                        driver's id, as \"user\", for what that driver sets working alone, \
                        or \"*\" for all of the map, whoever starts it. When not given, the \
                        first driver's slice that shows what `at` names, or, with no `at`, \
                        the drivers alone, each with how much of the map it sets working.",
                },
                "at": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Where to open the map, by the ids the map gives its \
                        nodes: the boxes to open, from the top down, as [\"app\", \
                        \"server\"], and last, if you will, a type to choose, as \
                        \"app/server/Server\", or a function of one, as \
                        \"app/server/Server::run\", whose detail the page shows. The last \
                        alone will do: the boxes above it open for it. A box that opens is \
                        opened, one that does not is chosen. The top when empty or not \
                        given.",
                },
            },
        },
    })
}

/// Asks coxd for the map of the project `arguments` name, or its first,
/// asks the chat's stage to show it where they say, and answers what is
/// there; or, when the project has no map, how to write one.
pub(super) async fn show_map(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let at: Vec<String> = match arguments.get("at") {
        None | Some(Value::Null) => Vec::new(),
        Some(at) => at
            .as_array()
            .ok_or("at is to be a list of ids, as strings")?
            .iter()
            .map(|id| {
                id.as_str()
                    .map(str::to_owned)
                    .ok_or("at is to be a list of ids, as strings")
            })
            .collect::<Result<_, _>>()?,
    };
    let text = |argument: &str, what: &str| match arguments.get(argument) {
        None | Some(Value::Null) => Ok(None),
        Some(given) => given
            .as_str()
            .map(|given| Some(given.to_owned()))
            .ok_or(format!("{argument} is to be {what}, as a string")),
    };
    let name = text("map", "a map's name")?;
    let driver = text("driver", "a driver's id")?;
    let (answer, drawn) = oneshot::channel();
    chats
        .send(ChatRequest::Map {
            folder: cwd.to_owned(),
            name: name.clone(),
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let told = drawn
        .await
        .map_err(|_| "the chat closed before the map was drawn".to_owned())??;
    let (at, text) = told_at(&told, name.as_deref(), driver.as_deref(), &at)?;
    ask(requests, ToolRequest::ShowMap { map: name, at })?;
    Ok(text)
}

/// What of a map is shown: all of it, or what one of its drivers sets
/// working, whose top and levels are its own.
#[derive(Clone, Copy)]
struct View<'m> {
    top: &'m Top,
    levels: &'m BTreeMap<String, Level>,
    slice: Option<&'m Slice>,
}

/// What of `map` is shown for `driver`, with what the page is told to
/// draw it for, a driver's id or [`EVERYTHING`]: the driver's slice, or
/// all of the map; with no driver given, the first slice that shows what
/// `at` names, or all of it when none does. Nothing for a map with
/// drivers when neither is given: its drivers alone are shown.
fn viewed<'m>(
    map: &'m Generated,
    driver: Option<&str>,
    at: &[String],
) -> Result<Option<(View<'m>, Option<&'m str>)>, String> {
    let whole = View {
        top: &map.top,
        levels: &map.levels,
        slice: None,
    };
    let of = |slice: &'m Slice| View {
        top: &slice.top,
        levels: &slice.levels,
        slice: Some(slice),
    };
    let drivers = || {
        map.slices
            .iter()
            .map(|slice| format!("`{}`", slice.driver))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match driver {
        Some(_) if map.slices.is_empty() => Err(
            "the map's top names no drivers, so it is shown whole: leave `driver` out".to_owned(),
        ),
        Some(EVERYTHING) => Ok(Some((whole, Some(EVERYTHING)))),
        Some(driver) => match map.slices.iter().find(|slice| slice.driver == driver) {
            Some(slice) => Ok(Some((of(slice), Some(slice.driver.as_str())))),
            None => Err(format!(
                "the map has no driver `{driver}`; its drivers are: {}, and `{EVERYTHING}` \
                 is all of it",
                drivers()
            )),
        },
        None if map.slices.is_empty() => Ok(Some((whole, None))),
        None => {
            let Some(last) = at.last() else {
                return Ok(None);
            };
            let node = type_of_function(map, last).map_or(last.as_str(), |ty| ty.id.as_str());
            let shows = |slice: &&Slice| {
                slice.top.center == node
                    || slice.top.left.iter().any(|id| id == node)
                    || slice.top.right.iter().any(|id| id == node)
                    || slice
                        .levels
                        .values()
                        .any(|level| level.parts.contains_key(node))
            };
            Ok(Some(match map.slices.iter().find(shows) {
                Some(slice) => (of(slice), Some(slice.driver.as_str())),
                None => (whole, Some(EVERYTHING)),
            }))
        }
    }
}

/// The drivers of `map`, as the agent is told them where none is chosen:
/// each with how much of the map it sets working.
fn drivers_told(map: &Generated) -> String {
    let mut text = String::from(
        "\nIts top names who starts the project working, and the map is shown for one of \
         them at a time, what that driver sets working. Show it again with `driver`:",
    );
    for slice in &map.slices {
        let name = map
            .nodes
            .get(&slice.driver)
            .map_or(slice.driver.as_str(), |node| node.name.as_str());
        let _ = write!(
            text,
            "\n`{}` {name}: {}, {} of {}, {} of them its own",
            slice.driver,
            boxes(slice.boxes),
            slice.types,
            counted(map.types.len(), "type"),
            slice.own.len()
        );
    }
    let _ = write!(text, "\n`{EVERYTHING}`: all of the map, whoever starts it");
    text
}

/// Whose slice of `map` is shown, as the agent is told it: how much of
/// the map the driver sets working, and the map's other drivers.
fn slice_told(map: &Generated, slice: &Slice) -> String {
    format!(
        "\nIt is shown as `{}` sets it working: {}, {} of {}, {} of them its own. The map's \
         other drivers: {}; `{EVERYTHING}` is all of it.",
        slice.driver,
        boxes(slice.boxes),
        slice.types,
        counted(map.types.len(), "type"),
        slice.own.len(),
        map.slices
            .iter()
            .filter(|other| other.driver != slice.driver)
            .map(|other| format!("`{}`", other.driver))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Where the map `told`, asked for as `name`, opens `at`, from the top
/// down, as `driver` sets it working, and what the agent is told of it
/// there; or why it cannot open there, as when it has no node `at`
/// names. What it opens at starts with whose slice it is, for a map
/// with drivers.
fn told_at(
    told: &Told,
    name: Option<&str>,
    driver: Option<&str>,
    at: &[String],
) -> Result<(Vec<String>, String), String> {
    let (Some(named), Some(file)) = (&told.name, &told.file) else {
        if let Some(name) = name
            && !told.maps.is_empty()
        {
            let names: Vec<&str> = told.maps.iter().map(|map| map.name.as_str()).collect();
            return Err(format!(
                "the project has no map `{name}`; it has: {}",
                names.join(", ")
            ));
        }
        return Ok((
            Vec::new(),
            format!(
                "The project has no map yet: the Map tab on the stage says so. Write one, and \
                 show it again. What the boxes are, and what goes in each, is for you and the \
                 human to decide for this project, as its COX.md may say: the map says what \
                 the parts of the project mean, for a reader who does not know it, not what \
                 its files are. {MAP_FORMAT}"
            ),
        ));
    };
    let which = if told.maps.len() > 1 {
        format!("The map `{named}` of the project")
    } else {
        "The map of the project".to_owned()
    };
    let file = file.strip_prefix(&told.root).unwrap_or(file).display();
    let Some(map) = told.map.as_ref().filter(|_| told.state != State::Invalid) else {
        return Ok((
            Vec::new(),
            format!(
                "{which} is on the stage, but its vocabulary, {file}, does not read, as the \
                 Map tab says too:\n{}\nFix it, and show it again. {MAP_FORMAT}",
                told.problems.join("\n")
            ),
        ));
    };
    let viewed = viewed(map, driver, at)?;
    let at = match viewed {
        Some((view, _)) => opened(map, view, at).map_err(|why| {
            if told.state == State::Ready {
                why
            } else {
                format!("{why}; not all the code is indexed yet, so its types may be missing")
            }
        })?,
        None => Vec::new(),
    };
    let mut text = format!(
        "{which} is on the stage, open at {}, for the human to see where things are.",
        where_(map, &at)
    );
    if !map.says.is_empty() {
        let _ = write!(text, "\nIt shows: {}", map.says);
    }
    let _ = write!(text, "{}", indexes(told));
    let shown = if let Some((view, _)) = viewed {
        if let Some(slice) = view.slice {
            let _ = write!(text, "{}", slice_told(map, slice));
        }
        let _ = write!(text, "\n{}", there(map, view, &told.changed, &at));
        shown(map, view, &at)
    } else {
        let _ = write!(text, "{}", drivers_told(map));
        map.top.left.iter().map(String::as_str).collect()
    };
    let _ = write!(text, "{}", drift(map, told, &file.to_string(), &shown));
    let at: Vec<String> = viewed
        .and_then(|(_, driver)| driver)
        .map(str::to_owned)
        .into_iter()
        .chain(at)
        .collect();
    let others: Vec<_> = told
        .maps
        .iter()
        .filter(|other| other.name != *named)
        .collect();
    if !others.is_empty() {
        let _ = write!(text, "\nThe project's other maps, which `map` shows:");
        for other in others {
            let _ = write!(text, "\n`{}`", other.name);
            if !other.says.is_empty() {
                let _ = write!(text, ": {}", other.says);
            }
        }
    }
    Ok((at, text))
}

/// The node each node `view` shows is a part of: the one that opens the
/// level it is in.
fn parents(view: View<'_>) -> BTreeMap<&str, &str> {
    let mut parents = BTreeMap::new();
    for (opens, level) in view.levels {
        for part in level.parts.keys() {
            parents.entry(part.as_str()).or_insert(opens.as_str());
        }
    }
    parents
}

/// The type of `map` whose function `key` is, if it is one.
fn type_of_function<'a>(map: &'a Generated, key: &str) -> Option<&'a Type> {
    let (ty, _) = key.rsplit_once("::")?;
    map.types
        .get(ty)
        .filter(|ty| ty.functions.iter().any(|function| function.key == key))
}

/// Where `at` opens `map`, as `view` shows it: the nodes from the top
/// down to the last it names, each opening the level the next is in, and
/// then the last; or why it does not, as when the map has no node of the
/// last, the view leaves it out, or the others are not on the way to it.
fn opened(map: &Generated, view: View<'_>, at: &[String]) -> Result<Vec<String>, String> {
    let Some(last) = at.last() else {
        return Ok(Vec::new());
    };
    let (node, function) = if map.nodes.contains_key(last) {
        (last.as_str(), None)
    } else if let Some(ty) = type_of_function(map, last) {
        (ty.id.as_str(), Some(last.clone()))
    } else {
        let boxes: Vec<&str> = map
            .nodes
            .iter()
            .filter(|(id, _)| !map.types.contains_key(*id))
            .map(|(id, _)| id.as_str())
            .collect();
        return Err(format!(
            "the map has no node or function `{last}`; its boxes are: {}",
            boxes.join(", ")
        ));
    };
    let parents = parents(view);
    let top = view.top;
    if let Some(slice) = view.slice
        && !parents.contains_key(node)
        && top.center != node
        && !top.left.iter().chain(&top.right).any(|id| id == node)
    {
        let others: Vec<String> = map
            .slices
            .iter()
            .filter(|other| {
                other
                    .levels
                    .values()
                    .any(|level| level.parts.contains_key(node))
            })
            .map(|other| format!("`{}`", other.driver))
            .collect();
        let elsewhere = if others.is_empty() {
            format!("no driver's slice has it: show it with the driver `{EVERYTHING}`")
        } else {
            format!("it is in what {} sets working", others.join(", "))
        };
        return Err(format!(
            "`{node}` is not in what `{}` sets working; {elsewhere}",
            slice.driver
        ));
    }
    let mut way = vec![node.to_owned()];
    while let Some(&parent) = parents.get(way[0].as_str()) {
        if way.iter().any(|on| on == parent) {
            break;
        }
        way.insert(0, parent.to_owned());
    }
    // The ids given on the way are on it, in order.
    let mut rest = way.iter();
    for id in &at[..at.len() - 1] {
        if !rest.any(|on| on == id) {
            return Err(format!(
                "`{id}` is not on the way to `{last}`, which opens at {}",
                way.iter()
                    .map(|on| format!("`{on}`"))
                    .collect::<Vec<_>>()
                    .join(" › ")
            ));
        }
    }
    way.extend(function);
    Ok(way)
}

/// The nodes the map shows where `at` opens it, in `view`: those of the
/// level it shows, and the one chosen.
fn shown<'a>(map: &'a Generated, view: View<'a>, at: &'a [String]) -> BTreeSet<&'a str> {
    let top = view.top;
    let Some(last) = at.last() else {
        return top
            .left
            .iter()
            .chain([&top.center])
            .chain(&top.right)
            .map(String::as_str)
            .collect();
    };
    let ty = map.types.get(last).or_else(|| type_of_function(map, last));
    let level = match ty {
        Some(ty) => Some(ty.owner.as_str()),
        None if view.levels.contains_key(last) => Some(last.as_str()),
        None => at.len().checked_sub(2).map(|parent| at[parent].as_str()),
    };
    let mut shown: BTreeSet<&str> = match level.and_then(|level| view.levels.get_key_value(level)) {
        Some((level, inside)) => inside
            .parts
            .keys()
            .map(String::as_str)
            .chain([level.as_str()])
            .collect(),
        None => [top.center.as_str()].into(),
    };
    shown.insert(last);
    shown.extend(ty.map(|ty| ty.id.as_str()));
    shown
}

/// Says in `told` how many of `all` were left out, past the first
/// `shown`, if any were.
fn and_more(told: &mut String, all: usize, shown: usize, what: &str) {
    if all > shown {
        let _ = write!(told, "\n- and {} more", counted(all - shown, what));
    }
}

/// The place `at` names, as the agent reads it.
fn where_(map: &Generated, at: &[String]) -> String {
    if at.is_empty() {
        return "its top".to_owned();
    }
    at.iter()
        .map(|id| match map.nodes.get(id) {
            Some(node) => format!("`{id}` {}", node.name),
            None => format!("`{id}`"),
        })
        .collect::<Vec<_>>()
        .join(" › ")
}

/// How the indexes the map is drawn from stand, a line for each that
/// is not ready, and whether the drift waits for them.
fn indexes(told: &Told) -> String {
    let mut text = String::new();
    for indexed in &told.indexes {
        let Some(line) = index_line(indexed) else {
            continue;
        };
        let _ = write!(text, "\n{line}");
    }
    if told.state != State::Ready {
        text.push_str("\nThe drift is told once all the code is indexed.");
    }
    text
}

/// How `indexed` stands, as the agent is told, unless it is ready.
fn index_line(indexed: &Indexed) -> Option<String> {
    let code = match (&indexed.language, indexed.directory.as_str()) {
        (Some(language), "") => format!("The {language} code"),
        (Some(language), directory) => format!("The {language} code of {directory}"),
        (None, "") => "The code".to_owned(),
        (None, directory) => format!("The code of {directory}"),
    };
    let tool = indexed.indexer.as_deref().unwrap_or("its indexer");
    let commit = indexed.commit.as_deref().map_or("", short);
    let mut line = match &indexed.state {
        IndexState::Ready => return None,
        IndexState::Indexing { .. } => {
            let meanwhile = match &indexed.drawn {
                Some(drawn) => format!("the map is drawn from {} meanwhile", short(drawn)),
                None => "its boxes show no types until it is done".to_owned(),
            };
            format!("{code} at {commit} is being indexed by {tool}: {meanwhile}.")
        }
        IndexState::Failed { reason } => format!(
            "{code} at {commit} could not be indexed by {tool}, which the Map tab tries \
             again: {reason}"
        ),
        IndexState::Missing { how } => format!("{code} is not in the map: {how}"),
        IndexState::Unknown { known } => format!(
            "{code} is not in the map: Cox indexes code of {} only.",
            known.join(", ")
        ),
        IndexState::Uncommitted { why } => format!("{code} is not in the map: {why}"),
    };
    if let Some(note) = &indexed.note {
        let _ = write!(line, " {note}");
    }
    Some(line)
}

/// `count` of `what`, as `1 type` or `3 types`.
fn counted(count: usize, what: &str) -> String {
    if count == 1 {
        format!("1 {what}")
    } else {
        format!("{count} {what}s")
    }
}

/// `count` boxes, as `1 box` or `3 boxes`.
fn boxes(count: usize) -> String {
    if count == 1 {
        "1 box".to_owned()
    } else {
        format!("{count} boxes")
    }
}

/// A commit, by the first characters of its name, as Git shortens it.
fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// What the map shows at `at`, in `view`, node by node, with what
/// `changed` in each.
fn there(
    map: &Generated,
    view: View<'_>,
    changed: &BTreeMap<String, Changed>,
    at: &[String],
) -> String {
    let Some(last) = at.last() else {
        let top = view.top;
        let mut text = String::from("At the top:");
        for id in top.left.iter().chain([&top.center]).chain(&top.right) {
            text.push_str(&node_line(map, view, changed, id));
        }
        text.push_str(&lines_told(map, &top.lines));
        return text;
    };
    if let Some(ty) = map.types.get(last) {
        let mut text = level_told(map, view, changed, &ty.owner);
        let _ = write!(text, "\nChosen: {}", type_told(ty));
        return text;
    }
    if let Some(ty) = type_of_function(map, last) {
        let mut text = level_told(map, view, changed, &ty.owner);
        let _ = write!(text, "\nChosen: {}", type_told(ty));
        if let Some(function) = ty.functions.iter().find(|function| function.key == *last) {
            let _ = write!(text, "\n{}", function_told(map, function));
        }
        return text;
    }
    if view.levels.contains_key(last) {
        return level_told(map, view, changed, last);
    }
    let parent = at.len().checked_sub(2).map(|parent| at[parent].as_str());
    let mut text = parent.map_or_else(
        || {
            let mut text = String::from("At the top:");
            text.push_str(&node_line(map, view, changed, &view.top.center));
            text
        },
        |parent| level_told(map, view, changed, parent),
    );
    let _ = write!(text, "\nChosen:{}", node_line(map, view, changed, last));
    text
}

/// The level inside `opens`, as `view` shows it and the agent is told
/// it: its nodes, then its lines.
fn level_told(
    map: &Generated,
    view: View<'_>,
    changed: &BTreeMap<String, Changed>,
    opens: &str,
) -> String {
    let name = map
        .nodes
        .get(opens)
        .map_or(opens, |node| node.name.as_str());
    let mut text = format!("Inside `{opens}` {name}:");
    let Some(level) = view.levels.get(opens) else {
        return text;
    };
    let ends = |id: &String| view.slice.is_some_and(|slice| slice.ends.contains(id));
    let mut parts: Vec<(&String, &[i32; 2])> = level.parts.iter().collect();
    // As the page draws them, from the top left.
    parts.sort_by_key(|(_, [x, y])| (*y, *x));
    for (id, _) in parts {
        match map.types.get(id) {
            Some(ty) => {
                let _ = write!(
                    text,
                    "\n`{id}` {} {}: {}, {}",
                    ty.kind,
                    ty.name,
                    counted(ty.fields.len(), "field"),
                    counted(ty.functions.len(), "function")
                );
                if !ty.doc.is_empty() {
                    let _ = write!(text, "; {}", ty.doc);
                }
                if ends(id) {
                    text.push_str("; another driver starts here, and the slice goes no further");
                }
                text.push_str(&changed_told(changed.get(id.as_str())));
            }
            None => text.push_str(&node_line(map, view, changed, id)),
        }
    }
    text.push_str(&lines_told(map, &level.lines));
    text
}

/// A node of `map`, on a line of its own, as `view` shows it and the
/// agent is told it.
fn node_line(
    map: &Generated,
    view: View<'_>,
    changed: &BTreeMap<String, Changed>,
    id: &str,
) -> String {
    let Some(node) = map.nodes.get(id) else {
        return format!("\n`{id}`: not in the map");
    };
    let mut line = format!("\n`{id}` {}", node.name);
    if let Some(code) = &node.code {
        let _ = write!(line, " ({code})");
    }
    if node.many {
        line.push_str(", several of it");
    }
    if let Some(level) = view.levels.get(id) {
        let types = level
            .parts
            .keys()
            .filter(|part| map.types.contains_key(*part))
            .count();
        if level.size.is_some() {
            let _ = write!(line, "; opens on its {}", counted(types, "type"));
        } else {
            line.push_str("; opens");
        }
    }
    if view
        .slice
        .is_some_and(|slice| slice.ends.iter().any(|end| end == id))
    {
        line.push_str("; what the driver sets working ends here");
    }
    line.push_str(&changed_told(changed.get(id)));
    let drift = map
        .drift
        .iter()
        .filter(|drift| drift.agreed.is_none() && drift.nodes.iter().any(|node| node == id))
        .count();
    if drift > 0 {
        let _ = write!(line, "; drift: {drift}");
    }
    line
}

/// What changed in a node, if anything did.
fn changed_told(changed: Option<&Changed>) -> String {
    changed.map_or_else(String::new, |changed| {
        format!(
            "; changed: +{} -{} in {}",
            changed.added,
            changed.removed,
            counted(changed.files, "file")
        )
    })
}

/// `lines`, as the agent is told them, with how often the code of their
/// ends meets.
fn lines_told(map: &Generated, lines: &[Line]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let told: Vec<String> = lines
        .iter()
        .map(|line| {
            let arrow = if line.both { "↔" } else { "→" };
            let count = map
                .edges
                .iter()
                .find(|edge| {
                    (edge.a == line.from && edge.b == line.to)
                        || (edge.a == line.to && edge.b == line.from)
                })
                .map_or_else(String::new, |edge| format!(" ({})", edge.count));
            let weak = if line.weak { ", weak" } else { "" };
            format!("`{}` {arrow} `{}`{count}{weak}", line.from, line.to)
        })
        .collect();
    format!(
        "\nLines, with how often the code of their ends meets: {}",
        told.join(", ")
    )
}

/// A type, as the agent is told it: what it is, where, and its members.
fn type_told(ty: &Type) -> String {
    let mut text = format!(
        "`{}` {} {}, at {}:{}",
        ty.id, ty.kind, ty.name, ty.file, ty.line
    );
    if !ty.doc.is_empty() {
        let _ = write!(text, ": {}", ty.doc);
    }
    let used = |uses: &[Use]| uses.iter().filter(|used| !used.test).count();
    for field in ty.fields.iter().take(MEMBERS_TOLD) {
        let writes = field
            .uses
            .iter()
            .filter(|used| used.writes && !used.test)
            .count();
        let _ = write!(
            text,
            "\n- field `{}`: {}; used in {}, written in {writes}",
            field.name,
            field
                .sig
                .strip_prefix(&format!("{}: ", field.name))
                .unwrap_or(&field.sig),
            counted(used(&field.uses), "place")
        );
    }
    and_more(&mut text, ty.fields.len(), MEMBERS_TOLD, "field");
    for function in ty.functions.iter().take(MEMBERS_TOLD) {
        let _ = write!(
            text,
            "\n- function `{}`: {}; called from {}, calls {}",
            function.name,
            function.sig,
            counted(used(&function.uses), "place"),
            function.calls.len()
        );
    }
    and_more(&mut text, ty.functions.len(), MEMBERS_TOLD, "function");
    text
}

/// A function, as the agent is told it once chosen: where it is called
/// from, what it calls, and the fields it uses.
fn function_told(map: &Generated, function: &Function) -> String {
    let mut text = format!(
        "The function `{}`: {}, at {}:{}",
        function.key, function.sig, function.file, function.line
    );
    if !function.doc.is_empty() {
        let _ = write!(text, "; {}", function.doc);
    }
    let _ = write!(text, "\nCalled from: {}", callers(&function.uses));
    for (kind, said) in [
        (CallKind::Function, "Calls"),
        (CallKind::Field, "Uses fields"),
    ] {
        let all = function
            .calls
            .iter()
            .filter(|call| call.kind == kind)
            .count();
        let mut calls: Vec<String> = function
            .calls
            .iter()
            .filter(|call| call.kind == kind)
            .take(CALLS_TOLD)
            .map(|call| {
                let on = call
                    .node
                    .as_ref()
                    .filter(|node| !map.types.contains_key(*node))
                    .map_or_else(String::new, |node| format!(" in `{node}`"));
                let writes = if call.writes { ", writes" } else { "" };
                format!("{}{on}{writes}", call.name)
            })
            .collect();
        if all > calls.len() {
            calls.push(format!("and {} more", all - calls.len()));
        }
        let _ = write!(
            text,
            "\n{said}: {}",
            if calls.is_empty() {
                "none".to_owned()
            } else {
                calls.join(", ")
            }
        );
    }
    text
}

/// Where `uses` call a function from, outside tests, a function once
/// with how often it calls, and how many places in tests do.
fn callers(uses: &[Use]) -> String {
    let mut by: Vec<(String, &Use, usize)> = Vec::new();
    for used in uses.iter().filter(|used| !used.test) {
        let key = used
            .function
            .clone()
            .unwrap_or_else(|| format!("{}|{}", used.module, used.within));
        match by.iter_mut().find(|(known, _, _)| *known == key) {
            Some((_, _, count)) => *count += 1,
            None => by.push((key, used, 1)),
        }
    }
    let tests = uses.iter().filter(|used| used.test).count();
    let mut told: Vec<String> = by
        .iter()
        .take(CALLERS_TOLD)
        .map(|(_, used, count)| {
            let times = if *count > 1 {
                format!(" ({count} times)")
            } else {
                String::new()
            };
            format!("{} at {}:{}{times}", used.within, used.file, used.line)
        })
        .collect();
    if by.len() > told.len() {
        told.push(format!(
            "and {} more",
            counted(by.len() - told.len(), "function")
        ));
    }
    match (told.is_empty(), tests) {
        (true, 0) => "nowhere".to_owned(),
        (true, _) => format!("tests alone, in {}", counted(tests, "place")),
        (false, 0) => told.join(", "),
        (false, _) => format!(
            "{}; and {} in tests",
            told.join(", "),
            counted(tests, "place")
        ),
    }
}

/// The drift of `map`, where its vocabulary, `file`, and the code
/// disagree, as the agent is told it to talk through with the human: that
/// on the nodes `shown` first, as many as [`DRIFT_TOLD`].
fn drift(map: &Generated, told: &Told, file: &str, shown: &BTreeSet<&str>) -> String {
    let (agreed, mut open): (Vec<_>, Vec<_>) =
        map.drift.iter().partition(|drift| drift.agreed.is_some());
    open.sort_by_key(|drift| !drift.nodes.iter().any(|node| shown.contains(node.as_str())));
    if open.is_empty() && agreed.is_empty() {
        return if told.state == State::Ready {
            "\nNo drift: the vocabulary and the code agree.".to_owned()
        } else {
            String::new()
        };
    }
    let mut text = String::new();
    if !open.is_empty() {
        let _ = write!(
            text,
            "\nDrift, where the vocabulary, {file}, and the code disagree, {}: talk each through \
             with the human, then fix the vocabulary, change the code, or agree to leave it, as \
             {{\"id\", \"why\"}} in the vocabulary's `agreed`.",
            open.len()
        );
        for drift in open.iter().take(DRIFT_TOLD) {
            let _ = write!(text, "\n- `{}`: {}", drift.id, drift.text);
            let sites: Vec<String> = drift
                .at
                .iter()
                .take(SITES)
                .map(|site| {
                    if site.within.is_empty() {
                        format!("{}:{}", site.file, site.line)
                    } else {
                        format!("{}:{} in {}", site.file, site.line, site.within)
                    }
                })
                .collect();
            if !sites.is_empty() {
                let _ = write!(text, "; at {}", sites.join(", "));
            }
        }
        if open.len() > DRIFT_TOLD {
            let _ = write!(
                text,
                "\n- and {} more, which the Map tab's Drift lists, as the next show_map at \
                 the boxes they are on tells",
                open.len() - DRIFT_TOLD
            );
        }
    }
    if !agreed.is_empty() {
        let _ = write!(text, "\nAgreed to leave as it is, {}:", agreed.len());
        for drift in agreed.iter().take(AGREED_TOLD) {
            let _ = write!(
                text,
                "\n- `{}`: {}",
                drift.id,
                drift.agreed.as_deref().unwrap_or_default()
            );
        }
        and_more(&mut text, agreed.len(), AGREED_TOLD, "agreement");
    }
    text
}

#[cfg(test)]
mod tests;
