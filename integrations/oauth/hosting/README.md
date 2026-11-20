# Public CookBook identity

This static Vercel project serves `public/client.json` at
`https://itscookbook.dev/client.json`. It contains public native-app metadata,
not credentials. The domain is a stable OAuth identity; keep its address and
ownership under maintainer control.

The approved document lists four local callback ports, authorization-code and
refresh-token grants, and no client secret. Vercel serves only the `public`
directory. There are no functions, dependencies, database, or Git-triggered
deployments. The root URL is not a product landing page.

For an approved deployment, link this directory to the existing `cookbook`
Vercel project in the maintainer's team and deploy from this directory with
`vercel deploy --prod`. Keep local `.vercel` and environment files out of Git
and deployment uploads. Verify the public URL returns HTTP 200,
`application/json`, valid HTTPS, no redirect, and the exact bytes in
`public/client.json` before changing the bundled profile or configuring an
installation. Do not use an authenticated preview URL as the client identity.

The adjacent [bundled profile](../client-identity.json) supplies the default for
future app builds. Existing installations can use the owner-only
`connection-clients configure` command described in the
[connection runbook](../../../docs/runbooks/connecting-services.md#maintainer-client-identity-setup).
Publishing this document does not connect accounts or approve tools. Named
provider sign-in and callback qualification remain separate checks.
