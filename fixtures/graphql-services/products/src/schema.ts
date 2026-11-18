import gql from "graphql-tag";

export const typeDefs = gql`
  type Query {
    product(id: ID!): Product
    node(id: ID!): Node
  }
  interface Node { id: ID! }
  type Product implements Node @key(fields: "id") { id: ID! title: String! }
`;
