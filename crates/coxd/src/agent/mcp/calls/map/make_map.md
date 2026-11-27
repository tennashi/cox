Make the map `{name}` of this project with me. What starts the project working: {drivers}.

A map tells a reader who does not know the project what its parts mean and how they work together. It is not a list of its files. It is told from whoever starts the project working, its drivers: the map shows, for one driver at a time, only what that driver sets working. You write the map's vocabulary, which says what the code cannot: who the drivers are, what the boxes are, which code each owns, where they sit, and the lines no code backs. Cox reads the rest from an index of the code, draws it, works out what each driver sets working, and marks drift wherever the vocabulary and the code disagree. The form of the vocabulary is at the end of this message.

Work in these steps, and tell me briefly what you found and decided at each.

1. Survey the project before you write anything.
   - Its repositories, the languages of each, and how each is built.
   - The programs it runs, and where each starts: binaries, servers, daemons, workers, command-line tools, apps, scripts that run on their own. Note how they talk to each other: sockets, HTTP, pipes, files, queues.
   - What starts it working: the people who use it, and what each uses it for; the systems outside it that call into it; the clocks, schedules and events that set it off.
   - The systems outside it that it calls: services, devices, other apps, the operating system, stores of data it does not own.
   - Read the README and any architecture notes, then the entry points, then the modules they reach. Read the module tree of each package, as the index names modules, so that you know every module you will have to give a box.

2. Settle the drivers with me, before you write any box.
   - A driver is whoever or whatever starts the project working: a person, by what they use it for, as "Person using it" or "Person building it"; a system outside that calls into it, as a webhook's sender; a clock or an event that sets it off.
   - One person with two purposes is two drivers when the purposes set different parts working, as using the project and building it. One person with two ways in for the same purpose is one driver, as someone who uses a desktop window and a phone page.
   - What the project calls, and what only answers it, is not a driver: it goes around the project, not among the drivers.
   - Propose the drivers to me, each with a name and one line on what it starts, and ask me whether they are the right ones. Most projects have two to five.
   - One map holds all the drivers. Code that only one of them reaches, as the build tasks, the tests and the stand-ins the tests run against, is in this map too, in boxes of its own.
   - Say in one line what the map shows, for `says`.

3. Write the top level. Put the project as one box in the centre. Name the drivers in `top.drivers`, each an `outside` node. Around the centre put the other people and outside systems from your survey, as `outside` nodes in `left` and `right`. Draw the lines between them and the centre by hand, each from the end that starts it: from a driver to the centre, from the centre to what it calls. `both: true` only draws the answer's way too. Keep the top to what a newcomer needs first: the drivers, and about three to ten nodes around the centre.

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
   - Use `files` only where a module holds several files that belong to different boxes, or where the index names no module for them, as for tests and build scripts that sit beside a package's code.
   - Leave no module unowned. The map reports an unowned module as drift only when the boxes use it often, so check the module list from your survey against your claims yourself. Generated code belongs to the box that uses it.
   - Do not leave a box empty. A box that owns no code and holds no boxes is drift.

6. Draw each driver's lines in, and the other lines no code backs. Lines between boxes that own code, or hold boxes that do, come from the code; writing one is a mistake the map reports. Write a line by hand only between processes, between the project and the people and systems around it, and between `outside` nodes.
   - A line goes from the end that starts it. What a driver sets working follows the lines from their `from` only, so a line written the wrong way round cuts the driver off, or lets it reach what it does not start.
   - Start at each driver: draw a line from it to the box that takes it first, the window a person looks at, the command a developer runs, the endpoint a system calls. Where it reaches the project through something outside, as a network or another tool, draw it through that.
   - Then draw the lines between the processes on from there, each from the one that asks: from the window to the server it talks to, from the server to the store it keeps, from the project to each system it calls.
   - Draw each line that leaves a box that opens on boxes on, in the level inside, from the part that starts it, and so on down to a box that owns code. A driver's walk follows a line from a box that owns code once the code comes into the box; a line left at the edge of a box of boxes it follows only where the driver comes in at all of that box.
   - When a line reaches a box that opens, carry it into the box, and do it with a rule first. Find in the code the types that handle what comes in on the line: the ones that implement the trait or interface, or subclass the class, that the request, the event or the drawing goes through (as a protocol's handler trait for a socket, a server's request handler for HTTP, a view trait for what a person sees), or the ones whose code uses the type or function the talk arrives as. End the line at the box with `{"box": ..., "implements": ...}` or `{"box": ..., "uses": ...}` naming it. The line then goes on, as the box opens, to the boxes and the types that rule picks, and follows the code as it moves. These types are where the driver comes in.
   - A line from a driver that no rule and no line inside carries on comes in at all of the box: every type in it. That is right for a small box that serves that driver alone, and too wide for a large one.
   - Draw the line on by hand, from the same node to the part it reaches on the level inside, only where no type or trait names what handles it.
   - Give a line `stops: true` where what the driver starts ends at the line's `to`, though the code or the lines would lead on: as from a build task to the app it starts, whose working is another driver's.

7. Place the levels. For each box that opens on boxes, give the level its `box`, place its `parts` in a compact grid (each part is 216 by 56): rows of the same columns, each row filled from the left before the next begins, with no cell left empty between parts, and the same step between columns and between rows, as 320 across and 200 down. Order the parts the way the talk goes, from the left, where the drivers are, to the right, where the systems the project calls are. Cox may trade the places of parts to shorten the lines, so it is the set of places that you choose. Place the nodes of the levels above around it with `outer` or `at`, and leave room for the lines to run between them. Within the grid, put parts that talk to each other in cells next to each other, and the ones that talk to the outside in the column nearest the side the outside node is on. For a driver's slice Cox closes the parts up over those the driver does not reach, on the same grid.

8. Show the map and work its drift down.
   - Write the vocabulary to the map's file, then call `show_map` with the map `{name}`. With no driver named it answers the drivers, each with how many boxes and types it sets working; with `driver: "*"` it shows all of the map.
   - Read what it answers: how the indexes stand, and the drift. Wait for the index if it is not ready yet, and show the map again.
   - Fix the vocabulary for each item of drift that is a mistake of the map: an unowned module, a claim that names nothing, a hand line that the code backs, a rule that picks nothing, a box with too many types. Then show it again.
   - Open the boxes with `show_map` and `driver: "*"` as you go, and check that each level reads as you meant: that the boxes say what they do, the lines go where the talk goes, and no level is crowded.
   - Repeat until the drift is down to the few items that are not mistakes of the map.

9. Check each driver's slice with me. For each driver, call `show_map` with that `driver`, and open the boxes it comes in at.
   - Tell me how many boxes and types the driver sets working, of how many, and which boxes it leaves out. Ask me whether that is what this driver really starts.
   - A slice that is nearly all of the map, for a driver that should start little, comes in too wide: a line into a whole large box, a line written the wrong way round, or a line that should stop. Narrow the line with a rule, turn it, or give it `stops: true`.
   - A slice that misses what the driver plainly starts lacks a line: from the driver to where it comes in, between two processes on the way, or on from a box of boxes to the part that starts it.
   - A type the walk ends at is where another driver comes in: the slice shows it and goes no further. Check that those are the places where one driver's work hands over to another's.
   - Change the lines, show the slice again, and go on until each slice is one I agree with.

10. Talk the rest through with me. For each item of drift that is left, tell me what it is and what you think: fix the vocabulary, change the code, or leave it. When we agree to leave one, record it in `agreed` with its id and why, and show the map once more.
