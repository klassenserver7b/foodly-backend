### Frontend

2 Modes
- ONLINE
	- Editing in sync mit server
- OFFLINE
	- App Speicher bzw. Browser Local Storage
	- Create change backlog 
		- submit to server when back online

### Backend

-  Rust backend in Docker container selfhostable
	- API → Websocket
- SQLite DB für Rezepte
	- Assets und Texte als Hashed links
- Live editing
- geteilte Rezepte
- Sync handling mit clients
	- Execute offline change backlog
		- Drop invalid changes