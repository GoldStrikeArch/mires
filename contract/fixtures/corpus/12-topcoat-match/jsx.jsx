// Family 12 -- Topcoat `match` with markup arms.
// JSX has no match. The Solid equivalents are a nested ternary chain (what the
// plugin sees) or a <Switch>/<Match> pair (not in builtIns here, so it would
// compile as an ordinary component). The chain is used because it is what the
// compiler actually has to reason about.

const simpleMatch = (
  <div>
    {status === "draft" ? (
      <span>Draft</span>
    ) : status === "published" ? (
      <a href="/posts">{post.title}</a>
    ) : status === "archived" ? (
      <span>Archived</span>
    ) : (
      ""
    )}
  </div>
);

// A match arm whose body is a block of several sibling nodes.
const multiNodeArm = (
  <div>
    {user
      ? [<h1>{user.name}</h1>, <p>Signed in</p>]
      : <a href="/login">Sign in</a>}
  </div>
);

// Attribute-position match: JSX can only pick a value, not a set of attributes.
const attrMatch = <article class={state === "open" ? "open" : undefined} />;
