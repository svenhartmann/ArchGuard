FROM node:22-alpine
WORKDIR /app
RUN printf '{"type":"module"}\n' > package.json
COPY --from=runtime . ./runtime
COPY . ./services/current
USER node
EXPOSE 8080
CMD ["node", "runtime/start.js"]
