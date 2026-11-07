// Family 03 -- attribute expressions.
// Derived from upstream __dom_fixtures__/attributeExpressions, reduced to the
// attribute forms Topcoat's view! can also express: a static attribute, a
// dynamic attribute value, a dynamic value read off a member expression, a
// literal boolean attribute, and one that maps to a DOM property.

const staticAttrs = <div id="main" class="base" />;

const dynamicValue = <h1 id={id}>Welcome</h1>;

const dynamicMember = <a href={post.url} title={post.title} />;

const booleanAttr = <input type="text" disabled />;

const mixed = (
  <div id="main">
    <a href={"/"} data-slug={post.slug}>
      Welcome
    </a>
  </div>
);

const property = <input value={state.value} />;
