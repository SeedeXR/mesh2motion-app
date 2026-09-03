import { Mesh2MotionEngine } from './Mesh2MotionEngine'

export class CustomModelUploadBootstrap {
  private readonly mesh2motion_engine: Mesh2MotionEngine

  constructor () {
    this.mesh2motion_engine = new Mesh2MotionEngine()
  }
}

// instantiate the class to setup event listeners
const app = new CustomModelUploadBootstrap()
