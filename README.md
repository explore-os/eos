# EOS System

This repo contains all the tooling behind [EOS](https://github.com/explore-os)

## 9P Filesystem Interface

EOS exposes its internal actor system state through a 9P2000.L filesystem interface. This allows you to inspect and modify the system state using standard filesystem operations.

### Filesystem Structure

```
/
├── spawn_queue       # Pending actor spawn requests (read-only)
└── actors/           # Directory of all actors
    └── {actor_id}/   # Directory for each actor
        ├── mailbox   # Actor's incoming message queue (writable)
        ├── send_queue # Actor's outgoing message queue (read-only)
        ├── script    # Path to actor's script (read-only)
        ├── state     # Actor's current state in JSON (writable)
        └── paused    # Actor's paused state as boolean (writable)
```

### Mounting the Filesystem

Using a 9P client (e.g., `9pfuse`):

```bash
9pfuse 'unix!/tmp/eos:0' /mnt/eos
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

Add a message to an actor's mailbox by writing a JSON message object:

```bash
# Simple message
echo '{"from":"sender_id","to":"receiver_id","payload":{"type":"greeting","message":"hello"}}' > /mnt/eos/actors/receiver_id/mailbox

# Message with complex payload
cat << 'EOF' > /mnt/eos/actors/my_actor/mailbox
{
  "from": "controller",
  "to": "my_actor",
  "payload": {
    "action": "update",
    "data": {
      "field1": "value1",
      "field2": 123
    }
  }
}
EOF
```

**Message Format:**
```json
{
  "from": "optional_sender_id",  // Optional: Can be null
  "to": "target_actor_id",       // Required: Must match the actor
  "payload": {                   // Required: Any JSON value
    "your": "data"
  }
}
```

**Requirements:**
- Content must be valid JSON
- Must contain `from`, `to`, and `payload` fields
- The message is appended to the actor's mailbox
- Actor will process it on the next tick

### Example Workflow

```bash
# Mount the filesystem
9pfuse 'unix!/tmp/eos:0' /mnt/eos

# List all actors
ls /mnt/eos/actors

# Check an actor's current state
cat /mnt/eos/actors/my_actor/state

# Update the actor's state
echo '{"count": 5, "ready": true}' > /mnt/eos/actors/my_actor/state

# Send a message to the actor
echo '{"from":"system","to":"my_actor","payload":{"cmd":"start"}}' > /mnt/eos/actors/my_actor/mailbox

# Pause the actor
echo 'true' > /mnt/eos/actors/my_actor/paused

# Verify it's paused
cat /mnt/eos/actors/my_actor/paused

# Unpause the actor
echo 'false' > /mnt/eos/actors/my_actor/paused
```

### Error Handling

Write operations may fail with the following errors:

- **EINVAL (Invalid Argument)**: The written data is not valid JSON or doesn't match the expected format
- **ENOENT (No Such File)**: The specified actor or file doesn't exist
- **EISDIR (Is a Directory)**: Attempted to write to a directory
- **EROFS (Read-Only File System)**: Attempted to write to a read-only file (e.g., `script`, `send_queue`)

### Programming with the Filesystem

You can also interact with the filesystem programmatically:

#### Python Example
```python
import json

# Update actor state
state = {"counter": 42, "active": True}
with open("/mnt/eos/actors/my_actor/state", "w") as f:
    json.dump(state, f)

# Send a message
message = {
    "from": "controller",
    "to": "my_actor",
    "payload": {"action": "process", "data": [1, 2, 3]}
}
with open("/mnt/eos/actors/my_actor/mailbox", "w") as f:
    json.dump(message, f)

# Read current state
with open("/mnt/eos/actors/my_actor/state", "r") as f:
    current_state = json.load(f)
    print(f"Current state: {current_state}")
```

#### Shell Script Example
```bash
#!/bin/bash

ACTOR_ID="my_actor"
MOUNT_POINT="/mnt/eos"

# Function to send a message
send_message() {
    local actor=$1
    local payload=$2
    echo "{\"from\":\"script\",\"to\":\"$actor\",\"payload\":$payload}" > "$MOUNT_POINT/actors/$actor/mailbox"
}

# Function to update state
update_state() {
    local actor=$1
    local state=$2
    echo "$state" > "$MOUNT_POINT/actors/$actor/state"
}

# Use the functions
send_message "$ACTOR_ID" '{"command":"start"}'
update_state "$ACTOR_ID" '{"status":"running","timestamp":1234567890}'
```

### Implementation Details

The write functionality is implemented in the `file_overlay.rs` module using the 9P2000.L protocol. Write operations acquire a write lock on the system to ensure thread-safe modifications.

#### Write Buffering

To support editors like `vi` that perform multiple write operations, the filesystem implements write buffering:

- When a file is opened for writing, writes are accumulated in a per-file buffer
- Multiple write operations at different offsets are properly handled
- The buffer is flushed to the actual system state on:
  - `fsync()` operations (when you save in vi with `:w`)
  - `close()` operations (when the file descriptor is closed)
- This ensures compatibility with standard text editors

#### Editor Compatibility

The filesystem supports both `fsync` and `setattr` operations required by editors like `vi`:

- **fsync**: Flushes buffered writes to the system immediately
- **setattr**: Handles attribute changes (timestamps, permissions) as no-ops

This means you can use `vi`, `nano`, `emacs`, or any standard editor to modify actor state:

```bash
# Edit actor state with vi
vi /mnt/eos/actors/my_actor/state

# Changes are buffered during editing and committed when you save and exit
```

#### Thread Safety

- Write operations acquire a write lock on the System (`sys.write().await`)
- Read operations use read locks to allow concurrent reads
- Buffering happens per-file descriptor (per-fid in 9P terms)
- Final commits are atomic at the file level

### Troubleshooting

#### "File has been changed" warning in vi

If you see this warning when saving in `vi`:
```
WARNING: The file has been changed since reading it!!!
Do you really want to write to it (y/n)?
```

**Solution**: This is normal behavior. Press `y` to continue. The warning occurs because the file's timestamp or attributes appear to change, but your edits will be saved correctly. The filesystem now properly handles `fsync` operations.

#### "Fsync failed" error

If you previously encountered:
```
E667: Fsync failed
```

This has been **fixed** in the current implementation. The filesystem now properly implements the `fsync` operation required by editors.

#### Write doesn't take effect immediately

**Expected behavior**: Writes are buffered until you:
- Save and exit the editor (`:wq` in vi)
- Explicitly save (`:w` in vi) - this triggers fsync
- Close the file descriptor

Changes won't be visible in the actor system until the buffer is flushed.

#### Invalid JSON error

If writes fail with "Invalid Argument" error:
```bash
# Check your JSON syntax
echo '{"valid": "json"}' > /mnt/eos/actors/my_actor/state  # ✓ Works

echo '{invalid json}' > /mnt/eos/actors/my_actor/state      # ✗ Fails
```

Use a JSON validator or `jq` to verify your JSON:
```bash
echo '{"test": 123}' | jq . > /mnt/eos/actors/my_actor/state
```

#### Actor doesn't exist error

If you get "No such file or directory":
- Verify the actor ID is correct: `ls /mnt/eos/actors`
- Ensure the actor hasn't been killed or removed
- Check spelling and case sensitivity

#### Permission denied

If writes fail with permission errors:
- Check filesystem mount permissions
- Verify the 9P client has write access
- Ensure you're writing to a writable file (not `script` or `send_queue`)

#### Buffered writes not appearing

If your writes don't seem to take effect:
1. Make sure you've closed the file or called fsync
2. Use `cat` to verify the file content after closing
3. Check system logs for parsing errors

Example to verify write worked:
```bash
echo '{"new": "state"}' > /mnt/eos/actors/my_actor/state
cat /mnt/eos/actors/my_actor/state  # Should show the new state
```
