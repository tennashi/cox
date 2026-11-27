Make the map `{name}` of this project with me, from {viewpoint}.

A map tells a reader who does not know the project what its parts mean and how they work together. It is not a list of its files. You write the map's vocabulary, which says what the code cannot: what the boxes are, which code each owns, where they sit, and the lines no code backs. Cox reads the rest from an index of the code, draws it, and marks drift wherever the vocabulary and the code disagree. The form of the vocabulary is at the end of this message.

Work in these steps, and tell me briefly what you found and decided at each.

1. Survey the project before you write anything.
   - Its repositories, the languages of each, and how each is built.
   - The programs it runs, and where each starts: binaries, servers, daemons, workers, command-line tools, apps, scripts that run on their own. Note how they talk to each other: sockets, HTTP, pipes, files, queues.
   - The people who use it, and in which role each meets it.
   - The systems outside it that it calls or that call it: services, devices, other apps, the operating system, stores of data it does not own.
   - Read the README and any architecture notes, then the entry points, then the modules they reach. Read the module tree of each package, as the index names modules, so that you know every module you will have to give a box.

2. Settle the viewpoint. The map shows the project from {viewpoint}. Say in one line what the map shows, for `says`. If the project has other maps, make this one show what they do not. Code that is not part of this viewpoint is left to another map: a map of what runs when someone uses the project leaves its build tasks, its tests and the stand-ins its tests run against to a map of how it is built, such as `build`. Say which code you leave out, and to which map.

3. Write the top level. Put the project as one box in the centre. Around it put the people and the outside systems from your survey, as `outside` nodes, the ones that drive it on the left and the ones it drives or depends on on the right. Draw the lines between them and the centre by hand, each one way or both, as the talk between them goes. Keep the top to what a newcomer needs first: about three to ten nodes around the centre.

4. Split the project into boxes, a level at a time.
   - Name each box in plain words for what it is or does to the person using the project, as "Window", "Payment checker", "Scheduler", capitalised as a title and without an article. Do not name a box after a module, a crate, a class or a folder. The module goes in `code`, under the name.
   - Keep a name short enough to draw: at most about 16 characters, which fit on one line. A longer name of two words or more, up to about 30 characters, is drawn on two lines and leaves less room for `code`; anything longer may be cut short. Keep `code` to about 30 characters too.
   - Pick the icon that reads best for each.
   - Use `many: true` for a box there are several of at run time, as one per connection or per document.
   - Inside the centre, start from the programs the survey found: one box for each process or app. Then split each box that owns too much into the parts a reader would name.
   - A box that owns code opens on its types. Keep each such box to a number of types a person can read on one level: roughly up to twenty, rarely more than thirty. When a box owns more, give it no code of its own, and split it into boxes on a level inside it that own that code. Nest as deep as the project needs. Large projects need two or three levels.
   - Keep each level readable too: about four to twelve boxes on it.

5. Give every piece of code an owner.
   - Use `claims` for the modules each box owns, as the index names them. A module goes to the box with the longest claim it is in, so a box can claim a package and a box inside it a module of it.
   - Use `files` only where a module holds several files that belong to different boxes, or where the index names no module for them.
   - Leave no module of this viewpoint unowned. The map reports an unowned module as drift only when the boxes use it often, so check the module list from your survey against your claims yourself. Generated code belongs to the box that uses it.
   - Code you left to another map needs no owner in this one, and no box made for it. Take care that no claim takes it in by the way: a claim of a whole package owns its tests and build scripts too, so claim the package's modules, or its source files with `files`, instead.
   - In a map that is not of the running project, as one of how it is built, the code the other maps show is still used by what this one shows. Give it one box that stays closed with `open: false`, and say in its `code` which map opens it.
   - Do not leave a box empty. A box that owns no code and holds no boxes is drift.

6. Draw lines only where no code backs them. Lines between boxes that own code, or hold boxes that do, come from the code; writing one is a mistake the map reports. Write a line by hand only between processes, between the project and the people and systems around it, and between `outside` nodes. When such a line reaches a box that opens, carry it into the box, and do it with a rule first:
   - Find in the code the types that handle what comes in on the line: the ones that implement the trait or interface, or subclass the class, that the request, the event or the drawing goes through (as a protocol's handler trait for a socket, a server's request handler for HTTP, a client type for a service the project calls), or the ones whose code uses the type or function the talk arrives as.
   - End the line at the box with `{"box": ..., "implements": ...}` or `{"box": ..., "uses": ...}` naming it. The line then goes on, as the box opens, to the boxes and the types that rule picks, and follows the code as it moves.
   - Pick a name narrow enough to reach the few types that really handle the line, in one or two boxes of each level. A trait that every view or widget implements, as focus, render or event traits of a UI framework, reaches most boxes and says little: draw the line from a person at the screen on by hand, to the box that takes their input first.
   - Draw the line on by hand, from the same node to the part it reaches on the level inside, only where no type or trait names what handles it.

7. Place the levels. For each box that opens on boxes, give the level its `box`, place its `parts` in a compact grid (each part is 216 by 56): rows of the same columns, each row filled from the left before the next begins, with no cell left empty between parts, and the same step between columns and between rows, as 320 across and 200 down. Order the parts the way the talk goes, from the left, where the people and systems that drive the project are, to the right, where those it drives are. Cox may trade the places of parts to shorten the lines, so it is the set of places that you choose. Place the nodes of the levels above around it with `outer` or `at`, and leave room for the lines to run between them. Within the grid, put parts that talk to each other in cells next to each other, and the ones that talk to the outside in the column nearest the side the outside node is on.

8. Show the map and work its drift down.
   - Write the vocabulary to the map's file, then call `show_map` with the map `{name}`.
   - Read what it answers: what the top shows, how the indexes stand, and the drift. Wait for the index if it is not ready yet, and show the map again.
   - Fix the vocabulary for each item of drift that is a mistake of the map: an unowned module, a claim that names nothing, a hand line that the code backs, a rule that picks nothing, a box with too many types. Then show it again.
   - Open the boxes with `show_map` as you go, and check that each level reads as you meant: that the boxes say what they do, the lines go where the talk goes, and no level is crowded.
   - Go through the lines you carried into boxes by hand, and turn each into a rule where a trait, an interface, a class or a type names what handles it. Check in `show_map` that the rule reaches the types you meant: a rule that picks nothing is drift, and one that reaches more than two boxes of a level is too wide.
   - Repeat until the drift is down to the few items that are not mistakes of the map.

9. Talk the rest through with me. For each item of drift that is left, tell me what it is and what you think: fix the vocabulary, change the code, or leave it. When we agree to leave one, record it in `agreed` with its id and why, and show the map once more.
