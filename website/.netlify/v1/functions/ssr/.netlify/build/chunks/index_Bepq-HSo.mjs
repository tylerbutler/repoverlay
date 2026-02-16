import { G as createVNode, h as Fragment, a1 as __astro_tag_component__ } from './astro/server_D6xwwJ9P.mjs';
import { d as $$CardGrid, e as $$Card } from './Code_BL1pyMJT.mjs';

const frontmatter = {
  "title": "repoverlay",
  "description": "Overlay config files into git repositories without committing them.",
  "template": "splash",
  "hero": {
    "tagline": "Overlay config files into git repositories without committing them. Symlink or copy shared configs — automatically excluded from git.",
    "actions": [{
      "text": "Get started",
      "link": "/quick-start/",
      "icon": "right-arrow",
      "variant": "primary"
    }, {
      "text": "What is repoverlay?",
      "link": "/introduction/",
      "variant": "secondary"
    }]
  }
};
function getHeadings() {
  return [{
    "depth": 2,
    "slug": "features",
    "text": "Features"
  }];
}
function _createMdxContent(props) {
  const {Fragment: Fragment$1} = props.components || ({});
  if (!Fragment$1) _missingMdxReference("Fragment");
  return createVNode(Fragment, {
    children: [createVNode(Fragment$1, {
      "set:html": "<div class=\"sl-heading-wrapper level-h2\"><h2 id=\"features\">Features</h2><a class=\"sl-anchor-link\" href=\"#features\"><span aria-hidden=\"true\" class=\"sl-anchor-icon\"><svg width=\"16\" height=\"16\" viewBox=\"0 0 24 24\"><path fill=\"currentcolor\" d=\"m12.11 15.39-3.88 3.88a2.52 2.52 0 0 1-3.5 0 2.47 2.47 0 0 1 0-3.5l3.88-3.88a1 1 0 0 0-1.42-1.42l-3.88 3.89a4.48 4.48 0 0 0 6.33 6.33l3.89-3.88a1 1 0 1 0-1.42-1.42Zm8.58-12.08a4.49 4.49 0 0 0-6.33 0l-3.89 3.88a1 1 0 0 0 1.42 1.42l3.88-3.88a2.52 2.52 0 0 1 3.5 0 2.47 2.47 0 0 1 0 3.5l-3.88 3.88a1 1 0 1 0 1.42 1.42l3.88-3.89a4.49 4.49 0 0 0 0-6.33ZM8.83 15.17a1 1 0 0 0 1.1.22 1 1 0 0 0 .32-.22l4.92-4.92a1 1 0 0 0-1.42-1.42l-4.92 4.92a1 1 0 0 0 0 1.42Z\"></path></svg></span><span class=\"sr-only\" data-pagefind-ignore>Section titled “Features”</span></a></div>\n"
    }), createVNode($$CardGrid, {
      children: [createVNode($$Card, {
        title: "Overlay without committing",
        icon: "rocket",
        "set:html": "<p>Apply shared config files to any git repo. Files are automatically excluded via <code dir=\"auto\">.git/info/exclude</code> — no <code dir=\"auto\">.gitignore</code> changes needed.</p>"
      }), createVNode($$Card, {
        title: "Symlink or copy",
        icon: "setting",
        "set:html": "<p>Files are symlinked by default for instant updates, or use <code dir=\"auto\">--copy</code> when symlinks aren’t suitable.</p>"
      }), createVNode($$Card, {
        title: "GitHub-native",
        icon: "github",
        "set:html": "<p>Pull overlays directly from GitHub repos, branches, tags, or subdirectories. Cached locally with shallow clones.</p>"
      }), createVNode($$Card, {
        title: "Fork inheritance",
        icon: "random",
        "set:html": "<p>Working on a fork? repoverlay automatically falls back to the upstream repo’s overlays when yours don’t exist yet.</p>"
      })]
    })]
  });
}
function MDXContent(props = {}) {
  const {wrapper: MDXLayout} = props.components || ({});
  return MDXLayout ? createVNode(MDXLayout, {
    ...props,
    children: createVNode(_createMdxContent, {
      ...props
    })
  }) : _createMdxContent(props);
}
function _missingMdxReference(id, component) {
  throw new Error("Expected " + ("component" ) + " `" + id + "` to be defined: you likely forgot to import, pass, or provide it.");
}

const url = "src/content/docs/index.mdx";
const file = "/home/tylerbu/code/claude-workspace/repoverlay/website/src/content/docs/index.mdx";
const Content = (props = {}) => MDXContent({
  ...props,
  components: { Fragment: Fragment, ...props.components, },
});
Content[Symbol.for('mdx-component')] = true;
Content[Symbol.for('astro.needsHeadRendering')] = !Boolean(frontmatter.layout);
Content.moduleId = "/home/tylerbu/code/claude-workspace/repoverlay/website/src/content/docs/index.mdx";
__astro_tag_component__(Content, 'astro:jsx');

export { Content, Content as default, file, frontmatter, getHeadings, url };
