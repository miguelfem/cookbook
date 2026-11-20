# Let a classifier learn from Other

This optional workflow helps when a useful new category keeps appearing among
items your classifier calls **Other**. Leave it off for recipes that do not need it.

With the Jev-enabled app connected to your agent, say:

> Create a CookBook classifier for these requests. Enable learning from Other.
> Collect examples, suggest categories when there is enough evidence, and evaluate
> them before changing the recipe. Show me the proposed change first.

Your agent creates the project/recipes and asks for the normal protected Jev
connection approval if needed. Account setup happens in CookBook, not in chat.
The installed MCP guide supplies the manifests and instructions automatically;
no repository checkout or manual project setup is needed.

1. **Use it normally.** Jev classifies requests; the recipe saves unmatched cases.
2. **Review patterns.** Ask your agent to review the saved Other cases. It proposes
   new categories supported by repeated examples. It can also conclude that no
   change is useful.
3. **Check the change.** Use fresh, independently labeled examples. CookBook tests
   both versions and reports improvements and regressions. This uses paid Jev calls
   through your approved connection, up to two per example.
4. **Choose the new version.** After a passing evaluation and your review, the agent
   registers the exact new recipe. Existing work keeps its original version.

To stop collecting, tell the agent to disable learning for that classifier.
Previously saved runs remain subject to normal history retention. This feature
does not keep a closed agent awake or silently schedule periodic changes.

The local fixture tests verify execution and rejection behavior using synthetic
provider responses. They do not establish Jev's accuracy on your data. For
high-impact decisions, choose a larger representative evaluation and domain review
before relying on automatic classification.
