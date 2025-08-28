POIS Server quick start

1) Ensure Cargo.toml has pinned versions (axum 0.7.5, sqlx 0.7.4, etc.).
2) Export env:
   export POIS_DB=sqlite://pois.db
   export POIS_ADMIN_TOKEN='dev-token'

3) Run:
   cargo run

4) Open UI:
   http://localhost:8080/
   Put your Bearer token into the toolbar.

5) Seed a default channel:
   curl -X POST http://localhost:8080/api/channels      -H "Authorization: Bearer dev-token" -H "Content-Type: application/json"      -d '{"name":"default"}'

6) Add a rule (example delete splice_insert):
   curl -X POST http://localhost:8080/api/channels/1/rules      -H "Authorization: Bearer dev-token" -H "Content-Type: application/json"      -d '{"name":"Delete splice_insert","priority":-1,"enabled":true,
          "match_json":{"anyOf":[{"scte35.command":"splice_insert"}]},
          "action":"delete","params_json":{}}'

7) Test /esam:
   curl -s http://localhost:8080/esam?channel=default      -H 'Content-Type: application/xml'      -d '<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1" xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0">
           <AcquiredSignal acquisitionSignalID="abc-123">
             <sig:UTCPoint utcPoint="2012-09-18T10:14:34Z"/>
           </AcquiredSignal>
         </SignalProcessingEvent>'
