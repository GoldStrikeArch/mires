// Family 11 -- Topcoat `if` / `else` with markup bodies.
// There is no JSX statement form, so the Solid equivalents are a ternary in
// child position and the <Show> built-in. Both are given: the ternary is the
// closer structural match (one insert, two branches), <Show> is what a Solid
// author would actually write.
//
// `Show` and `For` are left as free bindings rather than imported: the plugin's
// builtIns option already names them, and an unresolvable import would stop the
// trace at module load.

const ifElse = <div>{user.isSome ? <a href="/account">Account</a> : <a href="/login">Sign in</a>}</div>;

const ifOnly = <div>{user.isSome && <a href="/account">Account</a>}</div>;

const showBuiltIn = (
  <div>
    <Show when={user.isSome} fallback={<a href="/login">Sign in</a>}>
      <a href="/account">Account</a>
    </Show>
  </div>
);

const ifElseIf = (
  <div>{state.a ? <span>a</span> : state.b ? <span>b</span> : <span>fallback</span>}</div>
);

// Attribute-position `if`: view! can put a conditional in the attribute list.
// JSX has no such form; the nearest expressible thing is a conditional value
// per attribute.
const attrIf = <a href="/posts" aria-current={current ? "page" : undefined} />;
