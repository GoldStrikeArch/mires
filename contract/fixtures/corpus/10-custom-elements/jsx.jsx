// Family 10 -- custom elements.
// Derived from upstream __dom_fixtures__/customElements, reduced to the four
// forms that decide whether a name becomes an attribute or a property: an
// unknown dashed attribute, a camelCased one, the explicit attr: namespace, and
// the explicit prop: namespace.

const dashedAttr = <my-element some-attr={name} />;

const camelAttr = <my-element notProp={data} />;

const explicitAttr = <my-element attr:my-attr={data} />;

const explicitProp = <my-element prop:someProp={data} />;

const withChildren = (
  <my-element>
    <header slot="head">Title</header>
  </my-element>
);

const staticOnly = <my-widget data-widget-id="profile"></my-widget>;
