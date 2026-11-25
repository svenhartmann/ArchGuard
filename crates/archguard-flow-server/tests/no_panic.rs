//! The Go/PHP/Elixir/Ruby readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_common::extract_ctx::with_root_config;
use archguard_flow_server::{ui_flow_go, ui_flow_laravel, ui_flow_phoenix, ui_flow_rails};

const GO: &str = r#"
package main
import ("net/http"; "github.com/gin-gonic/gin")
func main() {
    r := gin.Default()
    g := r.Group("/api", AuthRequired())
    g.GET("/users/:id", getUser)
    http.HandleFunc("/h", func(w http.ResponseWriter, r *http.Request) { w.Write([]byte("é")) })
    r.Run(":8080")
}
"#;
const PHP: &str = r#"<?php
use Illuminate\Support\Facades\Route;
Route::middleware(['auth', 'can:admin'])->group(function () {
    Route::get('/users/{id}', [UserController::class, 'show'])->name('u');
    Route::resource('photos', PhotoController::class)->only(['index']);
});
Route::post('/p', fn () => "é");
"#;
const EX: &str = r#"
defmodule AppWeb.Router do
  use AppWeb, :router
  pipeline :auth do plug :require_user end
  scope "/", AppWeb do
    pipe_through [:browser, :auth]
    get "/users/:id", UserController, :show
    live "/l", PageLive, :index
    resources "/posts", PostController, only: [:index]
  end
end
"#;
const RB: &str = r#"
Rails.application.routes.draw do
  devise_for :users
  namespace :admin do
    resources :posts, only: [:index, :show] do member do get :preview end end
  end
  get '/h', to: 'home#index'
  root 'home#index'
end
class PostsController < ApplicationController
  before_action :authenticate_user!
  def index; @p = Post.all; end
end
"#;

#[test]
fn server_extractors_survive_broken_files() {
    let files = [
        (
            "go.mod",
            "module x\n\ngo 1.22\nrequire github.com/gin-gonic/gin v1.9.0\n",
        ),
        ("main.go", GO),
        (
            "composer.json",
            r#"{"require":{"laravel/framework":"^11"}}"#,
        ),
        ("routes/web.php", PHP),
        ("app/Http/Controllers/UserController.php", PHP),
        (
            "mix.exs",
            "defmodule X.MixProject do\n  defp deps, do: [{:phoenix, \"~> 1.7\"}]\nend\n",
        ),
        ("lib/app_web/router.ex", EX),
        ("Gemfile", "gem 'rails'\n"),
        ("config/routes.rb", RB),
        ("app/controllers/posts_controller.rb", RB),
    ];
    dirs::each_broken_tree(&files, |root| {
        ui_flow_go::extract(root);
        ui_flow_laravel::extract(root);
        ui_flow_rails::extract(root);
        with_root_config(root, ui_flow_phoenix::extract_with);
    });
}
