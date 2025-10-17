# ActOS

This project is an experimental **actor system built on the Linux filesystem and signals**.  
It acts like a **sandbox for exploring system behavior** - you can model services, send messages, and eventually pause,
inspect, and edit interactions at runtime. Think of it as a **lightweight debugger for distributed systems**.  

- [Introduction](#introduction)
- [Why?](#why)
- [How it works](#how-it-works)
- [Whats next](#whats-next)

---

## Introduction
This project is an experiment in building an **actor system on top of the Linux filesystem and signals**.
While it’s inspired by the actor model, you don’t need prior actor experience to understand the idea:  

- You can think of each **actor** as a service.  
- Sending a **message** is like calling a function or making a request to a server.  
- Whether you picture it as microservices, a monolith, or distributed servers, the same model applies.  

The goal is to create a **lightweight playground** where you can:  

- **Prototype microservice interactions**: watch requests and responses flow between simplified services.  
- **Experiment with execution order**: test “what happens if A runs before B” in a game or system.  
- **Inspect and modify at runtime** (planned): pause the system, edit messages, and resume execution.  

Think of it as turning a container into a **navigable debugger for your whole system**.  
It’s not production-ready and it’s slower than real-world setups—but that’s the point: it’s a safe,
transparent environment to explore how complex systems behave.  


## Why?
I was thinking about how to design a VM, for an actor based language, thats image based like smalltalk,
while still making it possible to use vscode (or any other editor for that matter) for editing the source code.
My first idea was to create a user mode file system with fuse that could map the code to files
in order for the editor to pick it up, but then I realized, that you could represent more than just source code.
For instance the internal state of actors. Then it dawned on me that you could also map all the messages in the
system like that and basically make everything explorable using any editor or basic unix tools like cd, ls and cat.

The only problem was, I have never made a file system with fuse and by the looks of it, it seems rather complicated.
So I decided to build a prototype that doesn't map the internal state of a VM onto a file system, but use the
file system to store its internal state instead.

You're probably already thinking of around a hundred reasons why this is a bad idea and I just want to say
that I agree. Using this in a production system would bring all sorts of problems with it.
It's inefficent, unsecure and probably many other things I can't think of right now.

So why build it? Well, as a learning tool. I thought it would be really cool if there was a system that isn't just
programmable, but also fully inspectable as well. And by building it on top of linux, it's possible to
use all the pre-existing tools to monitor and inspect the system while it's running.

## How it works
Currently there are two parts that make it run. The `supervisor` and `act`.

The `supervisor` is responsible for setting up the necessary directory structures, spawning actors/processes,
moving messages around and cleaning up when an actor dies.

`act` is the cli tool that is intended to simplify interacting with the `supervisor`.
It's technically not needed, but it should make it easier to get started.

When the `supervisor` starts, it creates the base directory structure in the specified root directory (actor system root, not linux root).
By default it puts everything into `/var/actors`, but you can change that by passing it as an argument on startup.
From now on, I'm going to to be using `$ACTOS_ROOT` when referring to that directory

There are three main directories:
- `$ACTOS_ROOT/spawn`
- `$ACTOS_ROOT/running`
- `$ACTOS_ROOT/send`

The `spawn` directory is for creating new actors. You just need to create a json file there containing the `path` to
the binary you want to run and the arguments (`args`) you want it to run with. After the file is created,
you need to notify the `supervisor` that a new actor is waiting to get created, which is done by sending it
the signal `USR1`. When spawning an actor through the `act` tool, this is done automatically, but if you want
to do it yourself, you can run `kill -USR1 $(cat $ACTOS_ROOT/.pid)`. Make sure to replace `$ACTOS_ROOT` with the actual directory.

When an actor is spawned, it gets passed its own ID, the path to its state file,
the path to the file it should read when a new message is available,
the directory it should write file into when sending a message and after that, all args specified in the json file.

After the actor has been spawned, you will see a new directory inside the `running` directory,
which contains all the data regarding this actor. It has a `state.json` containing its current state,
a `.pid` for coordination and a `spool` directory for working through messages.

An actor process should not hold internal state, but instead re-read its state from its state file before
handling a message. This makes it possible to manually edit the state when experimenting with the system.
After its done, it should save its state again so its up to date.

In order for the actor to actually get a message, it needs to be sent there. Thats what the `send` directory
is for. You just need to create a new json file with the contents of the message and use the correct naming scheme,
so that the `supervisor` knows where the messages needs to go. The naming scheme is as follows: `{ACTOR_ID}::RANDOM_ID.json`.
The random id is needed so that messages don't overwrite each other.

The `supervisor` scans the `send` directory in pre-defined intervals (2 second cycle by default),
if it sees a message with a valid name,
it will move the file into the actors `spool` directory, where its picked up in the next cycle.

The `supervisor` also scans the `spool` directories of each actor. If it doesn't contain a file named `current`,
it will take the oldest message and rename it to that. After thats done, it will send the signal `USR1` to
the actor, so it knows its allowed to read the message. After the actor is done, it will delete the file,
making space for the next one.

If the `supervisor` recieves a `KILL` signal, it will look through the `running` directory
and send `KILL` signals to every actor. Each actor has 30 seconds to cleanly terminate, after that its
directory will be removed.

Most common actions should be available through `act`, but if you want more control,
you can also build your own or write some scripts to interact with the system.

## Whats next
The whole thing is still quite rough around the edges. There are no supervision trees, which is
one of the great things about the actor model, so thats something I want to tackle in the future.
I also want to create a base actor that uses a script engine to make it simpler create your own
actors without having to worry about all the technical details.