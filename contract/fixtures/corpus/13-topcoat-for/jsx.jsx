// Family 13 -- Topcoat `for pat in expr` with a markup body.
// Solid spells this as the <For> built-in (keyed, reactive) or a plain .map()
// (unkeyed, re-created wholesale). Both are recorded: view!'s `for` is a
// server-side loop, so structurally it is the .map() that matches, while <For>
// is what a Solid author writes.

const mapped = (
  <ul>
    {posts.map(post => (
      <li>
        <a href={post.url}>{post.title}</a>
      </li>
    ))}
  </ul>
);

const forBuiltIn = (
  <ul>
    <For each={posts}>
      {post => (
        <li>
          <a href={post.url}>{post.title}</a>
        </li>
      )}
    </For>
  </ul>
);

const withFallback = (
  <ul>
    <For each={posts} fallback={<li>None</li>}>
      {post => <li>{post.title}</li>}
    </For>
  </ul>
);

// Attribute-position `for`: view! can emit zero or more attributes from a loop.
// JSX has no such form at all. A spread of a prebuilt object is the nearest
// thing, and it is a poor match: the spread is opaque to the compiler where
// view!'s loop body names each attribute. Kept as a spread of a bare binding
// rather than of a call, so the trace does not turn on how the stub handles
// iteration of a healed identifier.
const attrFor = <div {...attrs} />;
