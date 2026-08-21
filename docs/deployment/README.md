# AMOS deployment documentation

- [Customer-evaluation server](CUSTOMER_EVALUATION_SERVER.md): install and
  operate the current containerized application on a customer-controlled Linux
  server.

The current package is intentionally labeled `customer_evaluation`. It makes
the executable application installable without claiming completion of the
PostgreSQL, OIDC, production connector, isolated-worker, model-server, or cloud
object-store gates required for a production pilot.
