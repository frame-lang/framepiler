# Async

Some state machines need to do asynchronous work — network calls, file I/O, timers. Frame supports `async` declarations that generate async/await code in languages that support it. This chapter covers how async affects the dispatch chain and what changes in the generated code.

## Declaring Async Methods

Add `async` before interface methods, actions, or operations:

```
interface:
    async connect(url: str)
    async receive(): Message
    get_state(): str          # This one stays sync
```

## How Async Propagates

If *any* interface method is declared `async`, the **entire system** becomes async. All generated methods — including ones you declared as sync — will be async in the output.

This means callers must `await` every method on an async system, even `get_state()` above. This is a consequence of how state machines dispatch events internally: the system can't know at compile time which handler will run, so it must assume any call might be async.

Sync methods on an async system still work correctly — awaiting a synchronous function is a no-op in most languages.

## Example

```
@@target python_3

@@system HttpClient {
    interface:
        async fetch(url: str): str = ""
        get_last_url(): str = ""

    machine:
        $Idle {
            async fetch(url: str): str {
                self.last_url = url
                response = await http_get(url)
                -> $Done
                return response
            }
            get_last_url(): str {
                return self.last_url
            }
        }

        $Done {
            async fetch(url: str): str {
                self.last_url = url
                response = await http_get(url)
                return response
            }
            get_last_url(): str {
                return self.last_url
            }
        }

    actions:
        async http_get(url: str): str {
            import aiohttp
            async with aiohttp.ClientSession() as session:
                async with session.get(url) as resp:
                    return await resp.text()
        }

    domain:
        last_url: str = ""
}
```

The generated Python code uses `async def` for all dispatch methods and `await` for internal calls.

## Language Support

| Language | Async Support | Mechanism |
|----------|--------------|-----------|
| Python | Yes | `async def` / `await` |
| TypeScript | Yes | `async` / `await`, `Promise<T>` |
| Rust | Yes | `async fn` / `.await` |
| C | No | Warning emitted, `async` ignored |
| Go | Not needed | Goroutines handle concurrency without coloring |
| Java 21+ | Not needed | Virtual threads handle concurrency without coloring |

Languages like Go and Java don't need async/await — their concurrency models are "one-color," meaning any function can do concurrent work without special syntax. The `async` keyword is simply ignored for these targets.

## Two-Phase Initialization

Constructors can't be async in most languages. If your start state's enter handler (`$>`) needs to do async work, Frame generates a two-phase init:

1. The constructor creates the system and sets the initial state (sync)
2. A generated `init()` method fires the enter event (async)

```python
# Usage:
client = @@HttpClient()     # sync — just creates the object
await client.init()         # async — fires $Idle's $>() handler
await client.fetch("...")   # async — normal usage
```

## Try It

Add async `save()` and `load()` methods to the `Editor` example from Chapter 7. The `$Editing` state should handle both, writing/reading the buffer to/from a file.

[<- Previous: Hierarchical State Machines](08-hsm.md) | [Next: Advanced Topics ->](10-advanced.md)
