# EOS System

This repo contains all the tooling behind [EOS](https://github.com/explore-os)

## 9P Filesystem Interface

EOS exposes its internal actor system state through a 9P2000.L filesystem interface. This allows you to inspect and modify the system state using standard filesystem operations.

### Startup (without mounting)
```bash
eos serve
```

### Startup (with automatic mounting)

The following command will run the command to mount the filesystem at `/mnt/eos`:

```bash
eos serve /mnt/eos
```

_Note: eos tries running the mount command through sudo, so it may prompt for your password._

#### Filesystem Structure

```
/
├── spawn_queue       # Pending actor spawn requests (read-only)
└── actors/           # Directory of all actors
    └── {actor_id}/   # Directory for each actor
        ├── mailbox   # Actor's incoming message queue (writable)
        ├── script    # Path to actor's script (read-only)
        ├── state     # Actor's current state in JSON (writable)
        └── paused    # Actor's paused state as boolean (writable)
```

### Mounting the Filesystem (if not auto mounted)

```bash
sudo mount -t 9p -o version=9p2000.L,trans=unix,uname=$USER "$(eos sock)" /mnt/eos
```

### Read Operations

#### View All Actors
```bash
ls /mnt/eos/actors
```

#### Read Actor State
```bash
cat /mnt/eos/actors/{actor_id}/state
```

#### Check if Actor is Paused
```bash
cat /mnt/eos/actors/{actor_id}/paused
```

#### View Actor's Mailbox
```bash
cat /mnt/eos/actors/{actor_id}/mailbox
```

### Write Operations

The filesystem supports writing to specific files to modify the actor system's internal state.

#### Update Actor State

You can replace an actor's entire state by writing JSON to the `state` file:

```bash
# Simple state update
echo '{"counter": 42, "name": "example"}' > /mnt/eos/actors/{actor_id}/state

# Complex state with nested objects
cat << 'EOF' > /mnt/eos/actors/{actor_id}/state
{
  "counter": 100,
  "config": {
    "enabled": true,
    "threshold": 50
  },
  "data": ["item1", "item2", "item3"]
}
EOF
```

**Requirements:**
- Content must be valid JSON
- The entire state is replaced with the new JSON object
- Invalid JSON will result in an error

#### Pause/Unpause an Actor

Control whether an actor processes messages:

```bash
# Pause an actor
echo 'true' > /mnt/eos/actors/{actor_id}/paused

# Unpause an actor
echo 'false' > /mnt/eos/actors/{actor_id}/paused
```

**Requirements:**
- Value must be either `true` or `false` (case-sensitive)
- Paused actors will not process incoming messages

#### Send a Message to an Actor

```bash
# Simple message
eos send /mnt/eos/actors/receiver_id '{"type":"greeting","message":"hello"}'
```

**Requirements:**
- Content must be valid JSON

### Example Workflow

```bash
# Mount the filesystem
mkdir -p /mnt/eos
eos serve /mnt/eos

# List all actors
ls /mnt/eos/actors

# Check an actor's current state
cat /mnt/eos/actors/my_actor/state

# Update the actor's state
echo '{"count": 5, "ready": true}' > /mnt/eos/actors/my_actor/state

# Send a message to the actor
eos send /mnt/eos/actors/my_actor '{"cmd":"start"}'

# Pause the actor
echo 'true' > /mnt/eos/actors/my_actor/paused

# Verify it's paused
cat /mnt/eos/actors/my_actor/paused

# Unpause the actor
echo 'false' > /mnt/eos/actors/my_actor/paused
```
