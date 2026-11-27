class Var {
    void Run(Client client, string body) {
        client.LowLevel.Indices.Create<StringResponse>("cs-var", PostData.String(body));
    }
}
