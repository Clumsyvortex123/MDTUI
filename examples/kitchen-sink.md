---
title: mdtui kitchen sink
tags: [markdown, tui, demo]
---

# mdtui Kitchen Sink

A **fast** terminal markdown viewer with `vim` keys, *mouse support*, ~~no~~ live reload,
==highlights==, emoji shortcodes :rocket: :sparkles:, and [links](https://github.com/ratatui/ratatui).
Jump to [the tasks](#tasks) or open [another file](./plugins/README.md).

## Contents

- [Text](#text)
- [Tasks](#tasks)
- [Code](#code)
- [Tables](#tables)
- [Alerts](#alerts)
- [Images](#images)
- [Diagrams](#diagrams)
- [Math](#math)

## Text

Paragraphs wrap to the window width. Soft line breaks
join lines; a trailing backslash forces a hard\
break. Inline `code`, **bold**, *italic*, ***both***, ~~strike~~ and
autolinks like https://example.com all work. Footnotes too.[^speed]

> Block quotes render with a bar.
>
> > And they nest.

### Lists

1. First
2. Second
   - nested bullet
   - another
     - third level
3. Third

Term
: Description lists are supported as well.

## Tasks

- [x] Parse GFM with comrak
- [x] Render tables, alerts and footnotes
- [x] Render mermaid diagrams
- [ ] Lua plugins
  - [ ] nested task
	- [ ] another task

## Code

```rust
use std::collections::HashMap;

/// Count words in a document.
fn word_count(text: &str) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for word in text.split_whitespace() {
        *counts.entry(word).or_insert(0) += 1;
    }
    counts
}
```

```python
def fib(n: int) -> int:
    return n if n < 2 else fib(n - 1) + fib(n - 2)
```

```
plain fenced block with no language and a very long line that should wrap inside the box rather than overflow the terminal width
```

## Tables

| Feature     | Status  | Score |
|:------------|:-------:|------:|
| Markdown    | done    |    10 |
| Images      | done    |     9 |
| **Mermaid** | via mmdc |    8 |
| Plugins     | Lua 5.4 |   `∞` |

## Alerts

> [!NOTE]
> Alerts render with an icon and a colour bar.

> [!TIP]
> Press `?` for help.

> [!IMPORTANT]
> Checkbox clicks write back to the file.

> [!WARNING]
> Mermaid needs `mmdc` (or `mermaid-ascii`) on your PATH.

> [!CAUTION]
> Untrusted plugins can't run programs.

## Images

![gradient](images/gradient.png)

![logo](images/logo.svg) ![missing image](images/nope.png)

<p align="center"><img src="images/gradient.png" alt="html image" width="100"></p>

## Diagrams

```mermaid
flowchart LR
    A[Markdown] --> B(comrak AST)
    B --> C{IR}
    C --> D[Layout]
    D --> E[ratatui]
```

```mermaid
sequenceDiagram
    participant U as User
    participant M as mdtui
    U->>M: open README.md
    M-->>U: rendered preview
```

```mermaid
sequenceDiagram
    participant web as Web Browser
    participant blog as Blog Service
    participant account as Account Service
    participant mail as Mail Service
    participant db as Storage
Note over web,db: The user must be logged in to submit blog posts
    web->>+account: Logs in using credentials
    account->>db: Query stored accounts
    db->>account: Respond with query result
alt Credentials not found
        account->>web: Invalid credentials
    else Credentials found
        account->>-web: Successfully logged in
Note over web,db: When the user is authenticated, they can now submit new posts
        web->>+blog: Submit new post
        blog->>db: Store post data
par Notifications
            blog--)mail: Send mail to blog subscribers
            blog--)db: Store in-site notifications
        and Response
            blog-->>-web: Successfully posted
        end
    end
```

```mermaid
flowchart TD
    A[Open file] --> B{Changed on disk?}
    B -->|Yes| C[Re-render]
    B -->|No| D([Idle])
    C --> E[(Cache)]
    D -.-> B
```

```mermaid
stateDiagram-v2
    [*] --> Viewing
    Viewing --> Editing : e
    Editing --> Viewing : :w
    Editing --> [*] : :q!
```

```mermaid
pie title Where the time goes
    "Layout" : 45
    "Images" : 30
    "Diagrams" : 25
```

## Math

Inline math like $e^{i\pi} + 1 = 0$ and $\alpha^2 + \beta_1 \le \infty$.

$$
\sum_{i=1}^{n} i = \frac{n(n+1)}{2}
$$

---

<details>
<summary>HTML details</summary>

Raw HTML is shown as text.

</details>

[^speed]: Layout is cached per width; images decode on worker threads.
