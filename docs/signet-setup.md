For testing reasons, we have a compose file that will start a signet node and a
bridge node. The bridge node will connect to the signet node and will be
accessible from the host machine on port 38333 for P2P and 8080 for REST. This
is an easy way to test your Utreexo application using the signet.

## Requirements

- Docker
- Docker Compose

## Usage

```bash
$ docker-compose up
```

## Configuration

The compose file is configured to use the latest version of the bridge and
bitcoind. If you want to use a specific version, you can change the image tag
in the compose file.

You can also change the ports that are exposed to the host machine by changing
some environment variables, the available variables are:

 - API_PORT: The port that the bridge will listen to for REST requests (default: 8080)
 - P2P_PORT: The port that the bridge will listen to for P2P connections. e.g [florestad](https://github.com/Davidson-Souza/Floresta) or [utreexod](https://github.com/utreexo/utreexod) (default: 8333)
 - RPC_PORT: The port that the signet node will listen to for RPC requests (default: 38332)
 - RPC_USER: The username that will be used to authenticate RPC requests (default: user)
 - RPC_PASSWORD: The password that will be used to authenticate RPC requests (default: password)

You can change the environment variables by creating a `.env` file in the same
directory as the compose file and setting the variables there. For example:

```bash
export API_PORT=8080
export P2P_PORT=8333
export RPC_PORT=38332
export RPC_USER=user
export RPC_PASSWORD=password
```

and then running:

```bash
$ source .env
$ docker-compose up
```

## Troubleshooting

If you are having problems with the bridge, you can check the logs by running:

```bash
$ docker-compose logs -f bridge
```

If you are having problems with the signet node, you can check the logs by
running:

```bash
$ docker-compose logs -f signet
```