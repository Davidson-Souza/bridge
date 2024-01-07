import zmq

context = zmq.Context()
socket = context.socket(zmq.PULL)
socket.connect("tcp://127.0.0.1:5150")

for request in range(10):
    #  Get the reply.
    message = socket.recv()
    print("Received reply %s [ %s ]" % (request, message))
