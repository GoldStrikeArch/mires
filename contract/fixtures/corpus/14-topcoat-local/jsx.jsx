// Family 14 -- Topcoat `let pat = expr;` bindings inside a view body.
// JSX has no statement position, so the equivalent is an IIFE (a binding scoped
// to the markup, which is what view!'s `let` gives) or a const hoisted above
// the expression. Both are recorded.

const iife = (() => {
  const title = post.title.trim();
  return (
    <article>
      <h1>{title}</h1>
      <a href={post.url}>Read</a>
    </article>
  );
})();

const title = post.title.trim();
const hoisted = (
  <article>
    <h1>{title}</h1>
    <a href={post.url}>Read</a>
  </article>
);

// A binding used by an attribute that follows it in the same element.
const attrLocal = (() => {
  const href = post.url();
  return (
    <a href={href} data-slug={post.slug}>
      {post.title}
    </a>
  );
})();
