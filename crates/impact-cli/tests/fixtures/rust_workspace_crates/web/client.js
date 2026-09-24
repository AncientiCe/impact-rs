export function sendLeave(socket) {
  socket.send(Leave());
}

export function sendJoin(socket, room) {
  socket.send(Join(room));
}
